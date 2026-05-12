use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;
use futures_util::StreamExt;
use zbus::Connection;
use crate::constants::*;

pub(crate) enum GnomeDetectionMethod {
    /// Detected via D-Bus call to org.gnome.Shell.Extensions
    Dbus,
    /// Detected via gnome-extensions CLI and gsettings
    Cli,
}

pub(crate) struct GnomeExtensionStatus {
    pub(crate) installed: bool,
    pub(crate) enabled: bool,
    /// Extension is active in GNOME Shell (verified via D-Bus)
    pub(crate) active: bool,
    /// Whether org.gnome.Shell is reachable on the session bus.
    pub(crate) shell_service_available: bool,
    /// Raw state from D-Bus (None for CLI detection)
    /// 1=ENABLED, 2=DISABLED, 3=ERROR, 4=OUT_OF_DATE, 5=DOWNLOADING, 6=INITIALIZED
    pub(crate) state: Option<u8>,
    /// How the status was detected
    pub(crate) method: GnomeDetectionMethod,
}

pub(crate) fn gnome_state_name(state: u8) -> &'static str {
    match state {
        1 => "enabled",
        2 => "disabled",
        3 => "error",
        4 => "out_of_date",
        5 => "downloading",
        6 => "initialized",
        _ => "unknown",
    }
}

/// Parse GNOME Shell extension state from D-Bus response.
/// State values: 1.0=ENABLED, 2.0=DISABLED, 3.0=ERROR, 4.0=OUT_OF_DATE, 5.0=DOWNLOADING, 6.0=INITIALIZED
pub(crate) fn parse_gnome_extension_state(
    body: &HashMap<String, zbus::zvariant::OwnedValue>,
) -> GnomeExtensionStatus {
    // State is returned as f64 by GNOME Shell D-Bus API
    let state_f64: f64 = body
        .get("state")
        .and_then(|v| v.downcast_ref::<f64>().ok())
        .unwrap_or(0.0);
    let state = state_f64 as u8;

    // State 1 = ENABLED (active)
    let active = state == 1;

    GnomeExtensionStatus {
        installed: true,
        enabled: active,
        active,
        shell_service_available: true,
        state: Some(state),
        method: GnomeDetectionMethod::Dbus,
    }
}

pub(crate) enum GnomeDbusProbeResult {
    Status(GnomeExtensionStatus),
    ShellUnavailable,
    ProbeFailed,
}

pub(crate) fn is_dbus_service_unavailable(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, description, _) => {
            name.as_ref() == DBUS_ERROR_SERVICE_UNKNOWN
                || name.as_ref() == DBUS_ERROR_NAME_HAS_NO_OWNER
                || (name.as_ref() == DBUS_ERROR_UNKNOWN_METHOD
                    && description
                        .as_deref()
                        .map(|message| {
                            message.contains("Object does not exist at path")
                                || message.contains("No such interface")
                        })
                        .unwrap_or(false))
        }
        _ => false,
    }
}

/// Quick probe: check if extension is active via D-Bus call to GNOME Shell.
/// This bypasses filesystem searches and works reliably from systemd services.
pub(crate) fn gnome_extension_dbus_probe() -> GnomeDbusProbeResult {
    let connection = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[GNOME] D-Bus probe: failed to connect to session bus: {}",
                e
            );
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };
    gnome_extension_dbus_probe_with_connection(&connection)
}

/// Probe using a specific D-Bus connection (for testing with mock services)
pub(crate) fn gnome_extension_dbus_probe_with_connection(
    connection: &zbus::blocking::Connection,
) -> GnomeDbusProbeResult {
    let reply = match connection.call_method(
        Some(GNOME_SHELL_BUS_NAME),
        GNOME_SHELL_OBJECT_PATH,
        Some(GNOME_SHELL_EXTENSIONS_INTERFACE),
        "GetExtensionInfo",
        &(GNOME_EXTENSION_UUID,),
    ) {
        Ok(r) => r,
        Err(e) => {
            if is_dbus_service_unavailable(&e) {
                println!("[GNOME] D-Bus probe: GNOME Shell D-Bus interface not ready yet");
                return GnomeDbusProbeResult::ShellUnavailable;
            }
            eprintln!("[GNOME] D-Bus probe: GetExtensionInfo call failed: {}", e);
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };

    // Response is a dict (a{sv}) with extension info
    let body: HashMap<String, zbus::zvariant::OwnedValue> = match reply.body().deserialize() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[GNOME] D-Bus probe: failed to deserialize response: {}", e);
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };

    GnomeDbusProbeResult::Status(parse_gnome_extension_state(&body))
}

pub(crate) fn gnome_extension_status() -> GnomeExtensionStatus {
    // Quick probe: try D-Bus call to GNOME Shell first
    // This is the most reliable method from systemd services
    match gnome_extension_dbus_probe() {
        GnomeDbusProbeResult::Status(status) => return status,
        GnomeDbusProbeResult::ShellUnavailable => {
            return GnomeExtensionStatus {
                installed: false,
                enabled: false,
                active: false,
                shell_service_available: false,
                state: None,
                method: GnomeDetectionMethod::Dbus,
            };
        }
        GnomeDbusProbeResult::ProbeFailed => {}
    }

    // Fallback: CLI tools (may fail from systemd if XDG_DATA_DIRS is incomplete)

    // Check installed via gnome-extensions info (requires XDG_DATA_DIRS)
    let installed = Command::new("gnome-extensions")
        .args(["info", GNOME_EXTENSION_UUID])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // Check enabled via gsettings (more reliable from systemd services)
    let enabled = Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains(GNOME_EXTENSION_UUID)
        })
        .unwrap_or(false);

    GnomeExtensionStatus {
        installed,
        enabled,
        active: false,
        shell_service_available: true,
        state: None,
        method: GnomeDetectionMethod::Cli,
    }
}

pub(crate) fn wait_for_session_bus_name_owner(name: &'static str, timeout: Duration) -> bool {
    println!(
        "[GNOME] Waiting up to {}s for {} to appear on the session bus",
        timeout.as_secs(),
        name
    );

    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async move {
            let connection = match Connection::session().await {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to connect to session bus while waiting for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            let dbus = match zbus::fdo::DBusProxy::new(&connection).await {
                Ok(proxy) => proxy,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to create D-Bus proxy while waiting for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            if session_bus_name_has_owner(&dbus, name).await {
                println!("[GNOME] {} is already on the session bus", name);
                return true;
            }

            let mut owner_changes = match dbus
                .receive_name_owner_changed_with_args(&[(0, name)])
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to subscribe to NameOwnerChanged for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            if session_bus_name_has_owner(&dbus, name).await {
                println!("[GNOME] {} appeared on the session bus during setup", name);
                return true;
            }

            let wait_for_owner = async {
                while let Some(signal) = owner_changes.next().await {
                    let args = match signal.args() {
                        Ok(args) => args,
                        Err(error) => {
                            eprintln!(
                                "[GNOME] Failed to decode NameOwnerChanged for {}: {}",
                                name, error
                            );
                            continue;
                        }
                    };

                    if args.new_owner().is_some() {
                        println!("[GNOME] {} appeared on the session bus", name);
                        return true;
                    }
                }

                false
            };

            match tokio::time::timeout(timeout, wait_for_owner).await {
                Ok(has_owner) => has_owner,
                Err(_) => {
                    eprintln!(
                        "[GNOME] Timed out after {}s waiting for {} on the session bus",
                        timeout.as_secs(),
                        name
                    );
                    false
                }
            }
        })
    })
}

pub(crate) async fn session_bus_name_has_owner(proxy: &zbus::fdo::DBusProxy<'_>, name: &str) -> bool {
    match proxy.name_has_owner(name.try_into().unwrap()).await {
        Ok(has_owner) => has_owner,
        Err(error) => {
            eprintln!(
                "[GNOME] Failed to check D-Bus ownership for {}: {}",
                name, error
            );
            false
        }
    }
}
