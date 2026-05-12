pub(crate) mod detection;
pub(crate) mod install;
#[cfg(feature = "embed-gnome-extension")]
pub(crate) mod embed;

pub(crate) use detection::*;
pub(crate) use install::*;
#[cfg(feature = "embed-gnome-extension")]
pub(crate) use embed::*;

use std::time::Duration;
use crate::constants::*;

pub(crate) fn print_gnome_extension_install_instructions(reason: &str) {
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

pub(crate) fn print_gnome_extension_status(status: &GnomeExtensionStatus) {
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

pub(crate) fn ensure_gnome_extension(status: &GnomeExtensionStatus, auto_install: bool) -> bool {
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

pub(crate) fn setup_gnome_extension(auto_install: bool) {
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
