use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot, watch};
use uuid::Uuid;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

mod constants;
mod errors;
mod environ;
mod dbus_naming;
mod config;
mod focus;
mod args;
mod autostart;
mod broadcasters;
mod kanata;
mod control;
mod pause;
mod focus_pipeline;
mod lifecycle;
mod display_override;
mod supervisor;
mod backends;
mod sni;

use constants::*;
use errors::DynError;
use environ::*;
use dbus_naming::*;
use config::*;
use focus::*;
use args::*;
use autostart::*;
use broadcasters::*;
use kanata::*;
use control::*;
use control::client::*;
use pause::*;
use focus_pipeline::*;
use lifecycle::*;
use lifecycle::logind::*;
use lifecycle::startup::*;
use display_override::*;
use supervisor::*;
use backends::*;
use backends::wayland::*;
use sni::*;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::{constants::*, errors::*, environ::*, dbus_naming::*, config::*, focus::*, args::*, autostart::*, broadcasters::*, kanata::*, control::*, control::client::*, control::server::*, control::persistent::*, pause::*, focus_pipeline::*, lifecycle::*, lifecycle::logind::*, lifecycle::startup::*, display_override::*, supervisor::*, supervisor::capabilities::*, backends::*, backends::gnome::*, backends::kde::*, backends::kde::script::*, backends::kde::probe::*, backends::wayland::*, sni::*, sni::settings::*, sni::state::*, sni::indicator::*, sni::control_local::*, sni::control_dbus::*, sni::control_ops::*, sni::guard::*};



// === GNOME Extension Management ===

#[cfg(feature = "embed-gnome-extension")]
macro_rules! gnome_ext_file {
    ($file:literal) => {
        concat!("../../", "src/gnome-extension", "/", $file)
    };
}

#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_EXTENSION_JS: &str = include_str!(gnome_ext_file!("extension.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_METADATA_JSON: &str = include_str!(gnome_ext_file!("metadata.json"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_PREFS_JS: &str = include_str!(gnome_ext_file!("prefs.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_FORMAT_JS: &str = include_str!(gnome_ext_file!("format.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_DBUS_JS: &str = include_str!(gnome_ext_file!("dbus.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_FOCUS_JS: &str = include_str!(gnome_ext_file!("focus.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_DAEMON_STATE_JS: &str = include_str!(gnome_ext_file!("daemon-state.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_MULTIPLEX_JS: &str = include_str!(gnome_ext_file!("extension-multiplex.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_GSETTINGS_SCHEMA: &str = include_str!(gnome_ext_file!(
    "schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml"
));

fn get_gnome_extension_fs_path() -> PathBuf {
    let exe_path = env::current_exe().unwrap();
    let exe_dir = exe_path.parent().unwrap();
    exe_dir.join("gnome")
}

fn gnome_extension_fs_exists() -> bool {
    let path = get_gnome_extension_fs_path();
    path.join("extension.js").exists()
        && path.join("metadata.json").exists()
        && path.join("prefs.js").exists()
        && path.join("format.js").exists()
        && path.join("dbus.js").exists()
        && path.join("focus.js").exists()
        && path.join("daemon-state.js").exists()
        && path.join("extension-multiplex.js").exists()
        && path.join(GNOME_EXTENSION_SCHEMA_FILE).exists()
        && path.join(GNOME_EXTENSION_SCHEMA_COMPILED).exists()
}

#[cfg(feature = "embed-gnome-extension")]
fn compile_gnome_schemas(dir: &Path) -> std::io::Result<()> {
    let schema_dir = dir.join("schemas");
    let output = Command::new("glib-compile-schemas")
        .arg(&schema_dir)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "glib-compile-schemas failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(())
}

#[cfg(feature = "embed-gnome-extension")]
fn write_embedded_extension_to_dir(dir: &Path) -> std::io::Result<()> {
    fs::write(dir.join("extension.js"), EMBEDDED_EXTENSION_JS)?;
    fs::write(dir.join("metadata.json"), EMBEDDED_METADATA_JSON)?;
    fs::write(dir.join("prefs.js"), EMBEDDED_PREFS_JS)?;
    fs::write(dir.join("format.js"), EMBEDDED_FORMAT_JS)?;
    fs::write(dir.join("dbus.js"), EMBEDDED_DBUS_JS)?;
    fs::write(dir.join("focus.js"), EMBEDDED_FOCUS_JS)?;
    fs::write(dir.join("daemon-state.js"), EMBEDDED_DAEMON_STATE_JS)?;
    fs::write(dir.join("extension-multiplex.js"), EMBEDDED_MULTIPLEX_JS)?;
    let schema_dir = dir.join("schemas");
    fs::create_dir_all(&schema_dir)?;
    fs::write(
        dir.join(GNOME_EXTENSION_SCHEMA_FILE),
        EMBEDDED_GSETTINGS_SCHEMA,
    )?;
    compile_gnome_schemas(dir)?;
    Ok(())
}

enum GnomeDetectionMethod {
    /// Detected via D-Bus call to org.gnome.Shell.Extensions
    Dbus,
    /// Detected via gnome-extensions CLI and gsettings
    Cli,
}

struct GnomeExtensionStatus {
    installed: bool,
    enabled: bool,
    /// Extension is active in GNOME Shell (verified via D-Bus)
    active: bool,
    /// Whether org.gnome.Shell is reachable on the session bus.
    shell_service_available: bool,
    /// Raw state from D-Bus (None for CLI detection)
    /// 1=ENABLED, 2=DISABLED, 3=ERROR, 4=OUT_OF_DATE, 5=DOWNLOADING, 6=INITIALIZED
    state: Option<u8>,
    /// How the status was detected
    method: GnomeDetectionMethod,
}

fn gnome_state_name(state: u8) -> &'static str {
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
fn parse_gnome_extension_state(
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

enum GnomeDbusProbeResult {
    Status(GnomeExtensionStatus),
    ShellUnavailable,
    ProbeFailed,
}

fn is_dbus_service_unavailable(error: &zbus::Error) -> bool {
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
fn gnome_extension_dbus_probe() -> GnomeDbusProbeResult {
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
fn gnome_extension_dbus_probe_with_connection(
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

fn gnome_extension_status() -> GnomeExtensionStatus {
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

fn wait_for_session_bus_name_owner(name: &'static str, timeout: Duration) -> bool {
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

fn print_gnome_extension_install_instructions(reason: &str) {
    let fs_path = get_gnome_extension_fs_path();
    let install_steps = if gnome_extension_fs_exists() {
        format!(
            r#"Extension files are available at: {}

  gnome-extensions pack "{}" --force --out-dir=/tmp
  gnome-extensions install "/tmp/{}.shell-extension.zip" --force
  gnome-extensions enable {}"#,
            fs_path.display(),
            fs_path.display(),
            GNOME_EXTENSION_UUID,
            GNOME_EXTENSION_UUID
        )
    } else {
        format!(
            r#"Clone the repository and install:

  git clone https://github.com/7mind/kanata-switcher.git /tmp/kanata-switcher
  gnome-extensions pack /tmp/kanata-switcher/{} --force --out-dir=/tmp
  gnome-extensions install "/tmp/{}.shell-extension.zip" --force
  gnome-extensions enable {}"#,
            GNOME_EXTENSION_SRC_PATH, GNOME_EXTENSION_UUID, GNOME_EXTENSION_UUID
        )
    };

    eprintln!(
        r#"
[GNOME] Extension not installed.

{}

To install manually:

{}

Then restart GNOME Shell:
  - Press Alt+F2, type "r", press Enter (X11 only)
  - Or log out and log back in (Wayland)
"#,
        reason, install_steps
    );
}

fn pack_and_install_from_dir(src_dir: &Path, tmp_dir: &Path) -> Result<(), String> {
    let zip_name = format!("{}.shell-extension.zip", GNOME_EXTENSION_UUID);

    let pack_result = Command::new("gnome-extensions")
        .args([
            "pack",
            src_dir.to_str().unwrap(),
            "--force",
            &format!("--out-dir={}", tmp_dir.display()),
        ])
        .output();

    if pack_result.is_err() || !pack_result.as_ref().unwrap().status.success() {
        return Err("gnome-extensions pack failed".to_string());
    }

    let zip_path = tmp_dir.join(&zip_name);
    let install_result = Command::new("gnome-extensions")
        .args(["install", zip_path.to_str().unwrap(), "--force"])
        .output();

    if install_result.is_err() || !install_result.as_ref().unwrap().status.success() {
        return Err("gnome-extensions install failed".to_string());
    }

    Ok(())
}

#[allow(unused_variables, unused_assignments)]
fn install_gnome_extension() -> bool {
    let tmp_dir = tempfile::tempdir().unwrap();
    let fs_path = get_gnome_extension_fs_path();
    let mut fs_error: Option<String> = None;

    // Try filesystem first
    if gnome_extension_fs_exists() {
        println!("[GNOME] Installing from filesystem: {}", fs_path.display());
        match pack_and_install_from_dir(&fs_path, tmp_dir.path()) {
            Ok(()) => {
                println!("[GNOME] Extension installed");
                return true;
            }
            Err(e) => {
                eprintln!("[GNOME] Failed to install from filesystem: {}", e);
                fs_error = Some(e);
            }
        }
    } else {
        eprintln!(
            "[GNOME] Extension files not found at filesystem path: {}",
            fs_path.display()
        );
    }

    // Fallback to embedded extension
    #[cfg(feature = "embed-gnome-extension")]
    {
        eprintln!("[GNOME] Falling back to embedded extension...");
        let embedded_dir = tmp_dir.path().join("embedded");
        fs::create_dir_all(&embedded_dir).unwrap();

        if let Err(e) = write_embedded_extension_to_dir(&embedded_dir) {
            eprintln!("[GNOME] Failed to write embedded extension: {}", e);
            print_gnome_extension_install_instructions(
                "Auto-install failed: could not write embedded extension files.",
            );
            return false;
        }

        match pack_and_install_from_dir(&embedded_dir, tmp_dir.path()) {
            Ok(()) => {
                println!("[GNOME] Extension installed (from embedded)");
                return true;
            }
            Err(e) => {
                eprintln!("[GNOME] Failed to install from embedded: {}", e);
                print_gnome_extension_install_instructions(&format!("Auto-install failed: {}", e));
                return false;
            }
        }
    }

    #[cfg(not(feature = "embed-gnome-extension"))]
    {
        let reason = if let Some(e) = fs_error {
            format!(
                "Found extension files at {}, but installation failed: {}. \
                 Cannot fall back to embedded extension (disabled in this build).",
                fs_path.display(),
                e
            )
        } else {
            "Extension files not found and embedded extension is disabled in this build."
                .to_string()
        };
        print_gnome_extension_install_instructions(&reason);
        return false;
    }
}

fn enable_gnome_extension() -> bool {
    let result = Command::new("gnome-extensions")
        .args(["enable", GNOME_EXTENSION_UUID])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            println!("[GNOME] Extension enabled");
            true
        }
        _ => {
            eprintln!("[GNOME] Failed to enable extension");
            eprintln!("[GNOME] Try restarting GNOME Shell first:");
            eprintln!("[GNOME]   - Press Alt+F2, type \"r\", press Enter (X11 only)");
            eprintln!("[GNOME]   - Or log out and log back in (Wayland)");
            eprintln!(
                "[GNOME] Then run: gnome-extensions enable {}",
                GNOME_EXTENSION_UUID
            );
            false
        }
    }
}

fn ensure_gnome_extension(status: &GnomeExtensionStatus, auto_install: bool) -> bool {
    // If D-Bus probe confirmed extension is active, we're done
    if status.active {
        return false;
    }

    if !status.installed {
        if !auto_install {
            print_gnome_extension_install_instructions(
                "Auto-install was disabled (--no-install-gnome-extension).",
            );
            std::process::exit(1);
        }

        println!("[GNOME] Extension not installed, installing...");
        if !install_gnome_extension() {
            std::process::exit(1);
        }
    }

    if !status.enabled {
        println!("[GNOME] Extension not enabled, enabling...");
        if !enable_gnome_extension() {
            std::process::exit(1);
        }
        return true;
    }

    !status.installed
}

fn print_gnome_extension_status(status: &GnomeExtensionStatus) {
    let method_str = match status.method {
        GnomeDetectionMethod::Dbus => "via D-Bus",
        GnomeDetectionMethod::Cli => "via gnome-extensions",
    };

    if !status.shell_service_available {
        println!("[GNOME] Extension status: waiting for GNOME Shell D-Bus");
        return;
    }

    if status.active {
        println!("[GNOME] Extension status: active ({})", method_str);
    } else {
        let state_info = status
            .state
            .map(|s| format!(", state={}", gnome_state_name(s)))
            .unwrap_or_default();
        println!(
            "[GNOME] Extension status: {}, {} ({}{}){}",
            if status.installed {
                "installed"
            } else {
                "not installed"
            },
            if status.enabled {
                "enabled"
            } else {
                "not enabled"
            },
            method_str,
            state_info,
            if !matches!(status.state, Some(2) | Some(4)) {
                " - waiting for GNOME Shell..."
            } else {
                ""
            }
        );
    }
}

fn setup_gnome_extension(auto_install: bool) {
    // Retry settings for when extension is installed but GNOME Shell is still loading
    const RETRY_INTERVAL_MS: u64 = 50;
    const MAX_WAIT_MS: u64 = 30_000;
    const MAX_RETRIES: u64 = MAX_WAIT_MS / RETRY_INTERVAL_MS;
    const GNOME_SHELL_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

    let mut status = gnome_extension_status();
    print_gnome_extension_status(&status);

    if !status.shell_service_available {
        if !wait_for_session_bus_name_owner(GNOME_SHELL_BUS_NAME, GNOME_SHELL_WAIT_TIMEOUT) {
            print_gnome_extension_status(&status);
            std::process::exit(1);
        }

        let mut elapsed_ms: u64 = 0;
        loop {
            status = gnome_extension_status();
            if status.shell_service_available {
                print_gnome_extension_status(&status);
                break;
            }

            if elapsed_ms >= GNOME_SHELL_WAIT_TIMEOUT.as_millis() as u64 {
                print_gnome_extension_status(&status);
                std::process::exit(1);
            }

            std::thread::sleep(Duration::from_millis(RETRY_INTERVAL_MS));
            elapsed_ms += RETRY_INTERVAL_MS;

            if elapsed_ms % 1_000 == 0 {
                println!(
                    "[GNOME] Waiting for GNOME Shell D-Bus interface... ({}ms/{}ms)",
                    elapsed_ms,
                    GNOME_SHELL_WAIT_TIMEOUT.as_millis()
                );
            }
        }
    }

    // Retry on all states except:
    // - DISABLED (2): user explicitly disabled the extension
    // - OUT_OF_DATE (4): extension doesn't support current GNOME Shell version
    let is_transient_state = |s: Option<u8>| !matches!(s, Some(2) | Some(4));

    if status.installed && !status.active && is_transient_state(status.state) {
        let initial_state = status.state;
        let mut elapsed_ms: u64 = 0;
        for attempt in 0..MAX_RETRIES {
            std::thread::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS));
            elapsed_ms += RETRY_INTERVAL_MS;
            status = gnome_extension_status();

            if status.active {
                println!("[GNOME] Extension became active after {}ms", elapsed_ms);
                print_gnome_extension_status(&status);
                return;
            }

            if !is_transient_state(status.state) {
                println!(
                    "[GNOME] Extension state changed to {} after {}ms",
                    status.state.map(gnome_state_name).unwrap_or("unknown"),
                    elapsed_ms
                );
                break;
            }

            // Log progress every second
            if (attempt + 1) % 20 == 0 {
                println!(
                    "[GNOME] Still waiting for extension to load (state={})... ({}ms/{}ms)",
                    initial_state.map(gnome_state_name).unwrap_or("unknown"),
                    elapsed_ms,
                    MAX_WAIT_MS
                );
            }
        }

        if !status.active {
            print_gnome_extension_status(&status);
        }
    }

    let needs_restart = ensure_gnome_extension(&status, auto_install);

    if needs_restart {
        println!("[GNOME] Extension installed and enabled.");
        println!("[GNOME] Please restart GNOME Shell to activate the extension.");
        println!("[GNOME]   - Press Alt+F2, type \"r\", press Enter (X11 only)");
        println!("[GNOME]   - Or log out and log back in (Wayland)");
    }
}

// === Main ===

#[tokio::main]
async fn main() {
    loop {
        match run_once().await {
            Ok(RunOutcome::Restart) => {
                println!("[Restart] Restarting daemon");
            }
            Ok(RunOutcome::Exit) => break,
            Err(e) => {
                eprintln!("[Fatal] {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_once() -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    if args.install_autostart {
        install_autostart_desktop(&matches, &args)?;
        return Ok(RunOutcome::Exit);
    }
    if args.uninstall_autostart {
        uninstall_autostart_desktop()?;
        return Ok(RunOutcome::Exit);
    }
    if let Some(command) = resolve_control_command(&args) {
        let dispatch = match args.dbus_suffix.as_deref() {
            Some(suffix) => ControlDispatch::Unicast {
                bus_name: effective_dbus_name(suffix),
            },
            None => ControlDispatch::Broadcast,
        };
        send_control_command(command, dispatch).await?;
        return Ok(RunOutcome::Exit);
    }
    let effective_name = effective_dbus_name(&resolve_dbus_suffix(
        args.dbus_suffix.as_deref(),
        &args.host,
        args.port,
    )?);
    println!("[DBus] Using bus name: {}", effective_name);

    let install_gnome_extension = resolve_install_gnome_extension(&matches);

    let detected_env = detect_environment();
    println!("[Init] Detected environment: {}", detected_env.as_str());

    let config = load_config(args.config.as_deref());
    if config.rules.is_empty() && config.native_terminal_rule.is_none() {
        eprintln!("[Config] Error: No rules found in config file");
        eprintln!();
        eprintln!("Example config (~/.config/kanata/kanata-switcher.json):");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    let quiet_focus = args.quiet || args.quiet_focus;
    let status_broadcaster = StatusBroadcaster::new();
    let restart_handle = RestartHandle::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let shutdown_handle = ShutdownHandle::new();
    let runtime_handle = tokio::runtime::Handle::current();
    let kanata = KanataClient::new(
        &args.host,
        args.port,
        config.default_layer,
        args.quiet,
        status_broadcaster.clone(),
    );
    kanata.connect_with_retry().await;

    let focus_handler = Arc::new(Mutex::new(FocusHandler::new(
        config.rules.clone(),
        config.native_terminal_rule.clone(),
        quiet_focus,
    )));

    let lifecycle_provider = LifecycleProvider::build(detected_env).await;
    if !lifecycle_provider.is_continuous() && detected_env == Environment::Unknown {
        eprintln!("[Error] Could not detect display environment");
        eprintln!("[Error] login1 unavailable and no startup graphical environment detected");
        std::process::exit(1);
    }

    // Create shutdown guard - will switch to default layer when dropped
    let _shutdown_guard = ShutdownGuard::new(kanata.clone());
    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    let _persistent_dbus_service_guard = start_persistent_dbus_service(
        kanata.clone(),
        focus_handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
        runtime_environment.clone(),
        shutdown_handle.clone(),
        effective_name.clone(),
    );

    // Set up signal handlers
    let shutdown_handle_for_signal = shutdown_handle.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("failed to install SIGINT handler");
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler");

        tokio::select! {
            _ = sigterm.recv() => {
                eprintln!("[Signal] Received SIGTERM");
            }
            _ = sigint.recv() => {
                eprintln!("[Signal] Received SIGINT");
            }
            _ = sighup.recv() => {
                eprintln!("[Signal] Received SIGHUP");
            }
        }

        shutdown_handle_for_signal.request();
    });

    let enable_indicator = !args.no_indicator;
    if args.no_indicator {
        println!("[SNI] Indicator disabled via --no-indicator");
    }

    let _sni_guard = if enable_indicator {
        SniGuard::runtime_managed(
            runtime_environment.clone(),
            runtime_handle.clone(),
            kanata.clone(),
            focus_handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
            restart_handle.clone(),
            shutdown_handle.clone(),
            args.indicator_focus_only,
            effective_name.clone(),
        )
    } else {
        SniGuard::disabled()
    };

    let backend_context = BackendContext {
        kanata,
        handler: focus_handler,
        status_broadcaster,
        restart_handle: restart_handle.clone(),
        pause_broadcaster,
        runtime_environment,
        install_gnome_extension,
        gnome_setup_completed: Arc::new(AtomicBool::new(false)),
        gnome_setup_hook: Arc::new(setup_gnome_extension),
        effective_dbus_name: effective_name,
    };

    run_lifecycle_supervisor(
        lifecycle_provider,
        backend_context,
        restart_handle,
        shutdown_handle,
    )
    .await
}

// === Tests ===

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;
