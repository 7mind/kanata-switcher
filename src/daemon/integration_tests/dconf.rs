use super::*;

// === dconf Integration Tests ===

const DCONF_TEST_KEY: &str = "/org/gnome/shell/extensions/kanata-switcher/test-key";

fn is_dconf_available() -> bool {
    std::process::Command::new("dconf")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Guard that creates an isolated dconf environment using temp directories.
/// dconf reads directly from files, but writes go through dconf-service.
/// By setting XDG_RUNTIME_DIR to a temp dir, dconf-service (if available)
/// will use an isolated database. Falls back to direct file access.
struct IsolatedDconfEnv {
    _temp_dir: tempfile::TempDir,
    config_home: std::path::PathBuf,
    runtime_dir: std::path::PathBuf,
    profile_path: std::path::PathBuf,
}

impl IsolatedDconfEnv {
    fn new() -> Result<Self, String> {
        let temp_dir =
            tempfile::TempDir::new().map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let config_home = temp_dir.path().join("config");
        let runtime_dir = temp_dir.path().join("runtime");
        let dconf_dir = config_home.join("dconf");
        let profile_dir = temp_dir.path().join("dconf-profile");

        std::fs::create_dir_all(&dconf_dir)
            .map_err(|e| format!("Failed to create dconf dir: {}", e))?;
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("Failed to create runtime dir: {}", e))?;
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to create profile dir: {}", e))?;

        // Create a custom dconf profile that uses user-db in our temp location
        let profile_path = profile_dir.join("test");
        std::fs::write(&profile_path, "user-db:user\n")
            .map_err(|e| format!("Failed to write dconf profile: {}", e))?;

        Ok(Self {
            _temp_dir: temp_dir,
            config_home,
            runtime_dir,
            profile_path,
        })
    }

    fn dconf_read(&self, key: &str) -> Result<bool, String> {
        let output = std::process::Command::new("dconf")
            .args(["read", key])
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("DCONF_PROFILE", &self.profile_path)
            .output()
            .map_err(|e| format!("dconf read failed: {}", e))?;

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

    fn dconf_write(&self, key: &str, value: bool) -> Result<(), String> {
        let value_str = if value { "true" } else { "false" };
        let output = std::process::Command::new("dconf")
            .args(["write", key, value_str])
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("DCONF_PROFILE", &self.profile_path)
            .output()
            .map_err(|e| format!("dconf write failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("dconf write failed: {}", stderr.trim()));
        }

        Ok(())
    }
}

#[test]
fn test_dconf_write_and_read_bool_isolated() {
    if !is_dconf_available() {
        eprintln!("Skipping dconf test: dconf not available");
        return;
    }

    let env = IsolatedDconfEnv::new().expect("Failed to create isolated dconf env");

    // Write true
    let write_result = env.dconf_write(DCONF_TEST_KEY, true);
    assert!(
        write_result.is_ok(),
        "dconf write failed: {:?}",
        write_result
    );

    // Read back
    let read_result = env.dconf_read(DCONF_TEST_KEY);
    assert_eq!(read_result, Ok(true), "dconf read should return true");

    // Write false
    let write_result = env.dconf_write(DCONF_TEST_KEY, false);
    assert!(
        write_result.is_ok(),
        "dconf write failed: {:?}",
        write_result
    );

    // Read back
    let read_result = env.dconf_read(DCONF_TEST_KEY);
    assert_eq!(read_result, Ok(false), "dconf read should return false");

    // Temp dir is automatically cleaned up when env is dropped
}

#[test]
fn test_dconf_read_unset_key_isolated() {
    if !is_dconf_available() {
        eprintln!("Skipping dconf test: dconf not available");
        return;
    }

    let env = IsolatedDconfEnv::new().expect("Failed to create isolated dconf env");

    // Read unset key should return error (fresh isolated environment)
    let read_result = env.dconf_read(DCONF_TEST_KEY);
    assert!(
        read_result.is_err(),
        "Reading unset key should return error"
    );
    assert!(
        read_result.unwrap_err().contains("key not set"),
        "Error should indicate key not set"
    );
}

#[test]
fn test_sni_settings_store_with_isolated_dconf() {
    if !is_dconf_available() {
        eprintln!("Skipping dconf test: dconf not available");
        return;
    }

    let env =
        std::sync::Arc::new(IsolatedDconfEnv::new().expect("Failed to create isolated dconf env"));

    // Create a backend that uses the isolated environment
    struct IsolatedDconfBackend {
        env: std::sync::Arc<IsolatedDconfEnv>,
    }

    impl DconfBackend for IsolatedDconfBackend {
        fn get_bool(&self, key: &str) -> Result<bool, String> {
            self.env.dconf_read(key)
        }
        fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
            self.env.dconf_write(key, value)
        }
    }

    let backend = IsolatedDconfBackend { env: env.clone() };
    let mut store = SniSettingsStore::with_backend(Box::new(backend));

    // First read should fail (key not set in fresh env), but store should remain available
    let initial = store.read_focus_only();
    assert_eq!(initial, None);
    assert!(
        store.available,
        "Store should remain available after key-not-set error"
    );

    // Write should succeed
    store.write_focus_only(true);

    // Read should now return the written value
    let value = store.read_focus_only();
    assert_eq!(value, Some(true));

    // Toggle and verify
    store.write_focus_only(false);
    let value = store.read_focus_only();
    assert_eq!(value, Some(false));
}
