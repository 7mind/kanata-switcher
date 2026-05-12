use std::process::Command;
use crate::constants::DCONF_FOCUS_ONLY_KEY;

pub(crate) trait DconfBackend: Send + Sync {
    fn get_bool(&self, key: &str) -> Result<bool, String>;
    fn set_bool(&self, key: &str, value: bool) -> Result<(), String>;
}

pub(crate) struct ShellDconfBackend;

impl DconfBackend for ShellDconfBackend {
    fn get_bool(&self, key: &str) -> Result<bool, String> {
        dconf_get_bool(key)
    }

    fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
        dconf_set_bool(key, value)
    }
}

pub(crate) struct SniSettingsStore {
    pub(crate) available: bool,
    backend: Box<dyn DconfBackend>,
}

impl SniSettingsStore {
    pub(crate) fn new() -> Self {
        Self {
            available: true,
            backend: Box::new(ShellDconfBackend),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_backend(backend: Box<dyn DconfBackend>) -> Self {
        Self {
            available: true,
            backend,
        }
    }

    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self {
            available: false,
            backend: Box::new(ShellDconfBackend),
        }
    }

    pub(crate) fn read_focus_only(&mut self) -> Option<bool> {
        if !self.available {
            return None;
        }
        match self.backend.get_bool(DCONF_FOCUS_ONLY_KEY) {
            Ok(value) => Some(value),
            Err(error) => {
                if is_dconf_unavailable(&error) {
                    self.available = false;
                }
                eprintln!("[SNI] dconf read failed: {}", error);
                None
            }
        }
    }

    pub(crate) fn write_focus_only(&mut self, value: bool) {
        if !self.available {
            return;
        }
        if let Err(error) = self.backend.set_bool(DCONF_FOCUS_ONLY_KEY, value) {
            if is_dconf_unavailable(&error) {
                self.available = false;
            }
            eprintln!("[SNI] dconf write failed: {}", error);
        }
    }
}

pub(crate) fn dconf_get_bool(key: &str) -> Result<bool, String> {
    let output = Command::new("dconf")
        .args(["read", key])
        .output()
        .map_err(|error| format!("dconf read failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dconf read failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        "" => Err("key not set".to_string()),
        value => Err(format!("unexpected dconf output: {}", value)),
    }
}

pub(crate) fn dconf_set_bool(key: &str, value: bool) -> Result<(), String> {
    let value_str = if value { "true" } else { "false" };
    let output = Command::new("dconf")
        .args(["write", key, value_str])
        .output()
        .map_err(|error| format!("dconf write failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dconf write failed: {}", stderr.trim()));
    }

    Ok(())
}

pub(crate) fn is_dconf_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("no such file or directory")
        || lower.contains("not found")
        || lower.contains("failed to execute")
}
