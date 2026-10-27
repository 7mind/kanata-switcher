use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Check if dbus-daemon is available by trying to run it with --version
pub(crate) fn dbus_daemon_available() -> bool {
    std::process::Command::new("dbus-daemon")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub(crate) static DBUS_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Guard struct that starts a private dbus-daemon and cleans up on drop
pub(crate) struct DbusSessionGuard {
    child: std::process::Child,
    address: String,
    config_dir: std::path::PathBuf,
}

impl DbusSessionGuard {
    pub(crate) fn start() -> Result<Self, String> {
        if !dbus_daemon_available() {
            return Err("dbus-daemon binary not found in PATH".to_string());
        }

        // Create a minimal session config file with unique path per test
        let unique_id = DBUS_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let config_dir =
            std::env::temp_dir().join(format!("dbus-test-{}-{}", std::process::id(), unique_id));
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create config dir: {}", e))?;

        let config_path = config_dir.join("session.conf");
        let socket_path = config_dir.join("bus-socket");

        // Minimal session bus config
        let config_content = format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
            socket_path.display()
        );

        std::fs::write(&config_path, config_content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        // Start dbus-daemon with custom config
        let mut child = std::process::Command::new("dbus-daemon")
            .args([
                "--config-file",
                config_path.to_str().unwrap(),
                "--nofork",
                "--print-address",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn dbus-daemon: {}", e))?;

        // Read the address from stdout
        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture dbus-daemon stdout")?;
        let mut reader = std::io::BufReader::new(stdout);
        let mut address = String::new();
        std::io::BufRead::read_line(&mut reader, &mut address)
            .map_err(|e| format!("Failed to read dbus-daemon address: {}", e))?;
        let address = address.trim().to_string();

        if address.is_empty() {
            // Try to read stderr for error info
            if let Some(mut stderr) = child.stderr.take() {
                let mut err_output = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut err_output);
                let _ = child.kill();
                return Err(format!(
                    "dbus-daemon produced no address. stderr: {}",
                    err_output
                ));
            }
            let _ = child.kill();
            return Err("dbus-daemon produced no address".to_string());
        }

        // Wait for socket to be connectable (dbus-daemon ready)
        let socket_path_clone = socket_path.clone();
        wait_for(|| std::os::unix::net::UnixStream::connect(&socket_path_clone).ok())
            .map_err(|_| "Timeout waiting for dbus-daemon socket")?;

        Ok(Self {
            child,
            address,
            config_dir,
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for DbusSessionGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Clean up config directory
        let _ = std::fs::remove_dir_all(&self.config_dir);
    }
}
