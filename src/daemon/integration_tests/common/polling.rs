use super::*;
use std::time::{Duration, Instant};
use std::thread;

pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(50);
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const LONG_TEST_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) static WAYLAND_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) static DBUS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) static X11_FOCUS_QUERY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) static DISPLAY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct EnvVarGuard {
    pub(crate) key: &'static str,
    pub(crate) previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

/// Wait for a condition to become true, polling at 50ms intervals.
/// Returns Ok(T) when the condition returns Some(T), or Err after 1 minute timeout.
pub(crate) fn wait_for<T, F>(mut condition: F) -> Result<T, &'static str>
where
    F: FnMut() -> Option<T>,
{
    let start = Instant::now();
    while start.elapsed() < POLL_TIMEOUT {
        if let Some(result) = condition() {
            return Ok(result);
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err("Timeout waiting for condition")
}

/// Async version of wait_for for tokio tests
pub(crate) async fn wait_for_async<T, F, Fut>(mut condition: F) -> Result<T, &'static str>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = Instant::now();
    while start.elapsed() < POLL_TIMEOUT {
        if let Some(result) = condition().await {
            return Ok(result);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err("Timeout waiting for condition")
}

pub(crate) async fn with_test_timeout<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("test timeout")
}

pub(crate) async fn with_long_test_timeout<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(LONG_TEST_TIMEOUT, future)
        .await
        .expect("test timeout")
}
