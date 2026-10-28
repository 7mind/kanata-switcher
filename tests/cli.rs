#![cfg(target_os = "linux")]

use std::{io::{BufRead, BufReader}, process::{Command, Stdio}, sync::{Arc, Mutex}, time::Duration};

struct SessionBus(std::process::Child, String);
impl SessionBus {
    fn start() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped()).spawn().expect("install dbus-daemon");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut address).unwrap();
        Self(child, address.trim().into())
    }
}
impl Drop for SessionBus {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

struct Control(Arc<Mutex<Vec<&'static str>>>);
#[zbus::interface(name = "com.github.kanata.Switcher")]
impl Control {
    fn pause(&self) { self.0.lock().unwrap().push("pause"); }
    fn unpause(&self) { self.0.lock().unwrap().push("unpause"); }
    fn restart(&self) { self.0.lock().unwrap().push("restart"); }
}

async fn invoke(args: &[&str], config_home: &std::path::Path, bus: &str) -> std::process::Output {
    tokio::time::timeout(Duration::from_secs(5), tokio::process::Command::new(env!("CARGO_BIN_EXE_kanata-switcher"))
        .args(args).args(["--host", "127.0.0.1", "--port", "0"])
        .env("XDG_CONFIG_HOME", config_home).env("HOME", config_home)
        .env("DBUS_SESSION_BUS_ADDRESS", bus)
        .kill_on_drop(true).output()).await.expect("one-shot command waited for Kanata/config").unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_commands_work_without_config_or_kanata() {
    let bus = SessionBus::start();
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let _service = zbus::connection::Builder::address(bus.1.as_str()).unwrap()
        .name("com.github.kanata.Switcher.instances.cli_test").unwrap()
        .serve_at("/com/github/kanata/Switcher", Control(calls.clone())).unwrap()
        .build().await.unwrap();
    for (flag, method) in [("--pause", "pause"), ("--unpause", "unpause"), ("--restart", "restart")] {
        // Test both broadcast and suffix-directed routing against the real session bus.
        for suffix in [false, true] {
            let mut args = vec![flag, "--config", "/nonexistent/kanata-switcher.json"];
            if suffix { args.extend(["--dbus-suffix", "cli_test"]); }
            let output = invoke(&args, temp.path(), &bus.1).await;
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(calls.lock().unwrap().pop(), Some(method));
        }
    }
}

#[tokio::test]
async fn autostart_commands_work_without_config_or_kanata() {
    let temp = tempfile::tempdir().unwrap();
    let desktop = temp.path().join("autostart/kanata-switcher.desktop");
    for flag in ["--install-autostart", "--uninstall-autostart"] {
        let output = invoke(&[flag, "--config", "/nonexistent/kanata-switcher.json"], temp.path(), "unix:path=/nonexistent").await;
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(desktop.exists(), flag == "--install-autostart");
    }
}
