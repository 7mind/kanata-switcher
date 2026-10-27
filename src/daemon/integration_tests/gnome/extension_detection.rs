use super::*;

// === GNOME Shell Extension Detection Integration Tests ===

/// Mock GNOME Shell Extensions D-Bus service.
/// Implements GetExtensionInfo to verify the daemon probes correctly.
struct MockGnomeShellExtensions {
    /// Extension state to return (1.0=ENABLED, 2.0=DISABLED, etc.)
    state: f64,
}

#[zbus::interface(name = "org.gnome.Shell.Extensions")]
impl MockGnomeShellExtensions {
    /// Mock implementation of GetExtensionInfo
    /// Returns a{sv} dict with extension info including state as f64
    fn get_extension_info(&self, uuid: &str) -> HashMap<String, zbus::zvariant::OwnedValue> {
        use zbus::zvariant::{OwnedValue, Value};
        let mut info = HashMap::new();
        info.insert(
            "uuid".to_string(),
            OwnedValue::try_from(Value::Str(uuid.into())).unwrap(),
        );
        // GNOME Shell returns state as f64 - this is the critical detail we're testing
        info.insert(
            "state".to_string(),
            OwnedValue::try_from(Value::F64(self.state)).unwrap(),
        );
        info
    }
}

/// Integration test for GNOME extension D-Bus probe.
///
/// This test verifies:
/// 1. The probe calls the correct D-Bus destination (org.gnome.Shell)
/// 2. The probe uses the correct object path (/org/gnome/Shell)
/// 3. The probe calls the correct interface (org.gnome.Shell.Extensions)
/// 4. The probe calls the correct method (GetExtensionInfo)
/// 5. The probe correctly parses f64 state values (both enabled and disabled)
///
/// If any of these regress (wrong path, wrong type parsing, etc.),
/// this test will fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_gnome_extension_dbus_probe_integration() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        // Start private dbus-daemon
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // --- Test 1: Extension ENABLED (state=1.0) ---
        {
            let mock_service = MockGnomeShellExtensions { state: 1.0 };

            let service_connection = Builder::address(address.clone())
                .expect("Failed to create connection builder")
                .name(GNOME_SHELL_BUS_NAME)
                .expect("Failed to set bus name")
                .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
                .expect("Failed to serve mock service")
                .build()
                .await
                .expect("Failed to build connection");

            // Wait for service to be registered
            let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
                .await
                .unwrap();
            wait_for_async(|| {
                let proxy = dbus_proxy.clone();
                async move {
                    proxy
                        .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                        .await
                        .ok()
                        .filter(|&has_owner| has_owner)
                }
            })
            .await
            .expect("Timeout waiting for mock GNOME Shell registration");

            // Create blocking connection for the probe
            let client_connection = zbus::blocking::connection::Builder::address(address.clone())
                .expect("Failed to create client builder")
                .build()
                .expect("Failed to connect client");

            // Call the actual probe function - this verifies the full integration
            let status = match gnome_extension_dbus_probe_with_connection(&client_connection) {
                GnomeDbusProbeResult::Status(status) => status,
                GnomeDbusProbeResult::ShellUnavailable => {
                    panic!("D-Bus probe unexpectedly reported shell unavailable")
                }
                GnomeDbusProbeResult::ProbeFailed => {
                    panic!("D-Bus probe unexpectedly failed against mock service")
                }
            };

            // Verify the probe succeeded and correctly parsed the response
            assert!(status.active, "Extension with state=1.0 should be active");
            assert!(status.enabled, "Extension with state=1.0 should be enabled");
            assert!(
                status.installed,
                "Extension found via D-Bus should be marked installed"
            );
            assert!(matches!(status.method, GnomeDetectionMethod::Dbus));

            // Drop connections to release the bus name
            drop(client_connection);
            drop(service_connection);
        }

        // Small delay to ensure bus name is released
        tokio::time::sleep(Duration::from_millis(100)).await;

        // --- Test 2: Extension DISABLED (state=2.0) ---
        {
            let mock_service = MockGnomeShellExtensions { state: 2.0 };

            let service_connection = Builder::address(address.clone())
                .expect("Failed to create connection builder")
                .name(GNOME_SHELL_BUS_NAME)
                .expect("Failed to set bus name")
                .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
                .expect("Failed to serve mock service")
                .build()
                .await
                .expect("Failed to build connection");

            let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
                .await
                .unwrap();
            wait_for_async(|| {
                let proxy = dbus_proxy.clone();
                async move {
                    proxy
                        .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                        .await
                        .ok()
                        .filter(|&has| has)
                }
            })
            .await
            .expect("Timeout");

            let client_connection = zbus::blocking::connection::Builder::address(address.clone())
                .expect("Builder")
                .build()
                .expect("Connect");

            let status = match gnome_extension_dbus_probe_with_connection(&client_connection) {
                GnomeDbusProbeResult::Status(status) => status,
                GnomeDbusProbeResult::ShellUnavailable => {
                    panic!("Probe unexpectedly reported shell unavailable")
                }
                GnomeDbusProbeResult::ProbeFailed => panic!("Probe unexpectedly failed"),
            };

            assert!(
                !status.active,
                "Extension with state=2.0 should NOT be active"
            );
            assert!(
                !status.enabled,
                "Extension with state=2.0 should NOT be enabled"
            );
        }
    })
    .await;
}

/// Mock GNOME Shell Extensions D-Bus service with mutable state.
/// Used to test the retry logic when extension state changes during startup.
struct MockGnomeShellExtensionsDelayed {
    /// Extension state (atomic for cross-thread mutation)
    state: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[zbus::interface(name = "org.gnome.Shell.Extensions")]
impl MockGnomeShellExtensionsDelayed {
    fn get_extension_info(&self, uuid: &str) -> HashMap<String, zbus::zvariant::OwnedValue> {
        use zbus::zvariant::{OwnedValue, Value};
        let state_bits = self.state.load(std::sync::atomic::Ordering::SeqCst);
        let state = f64::from_bits(state_bits);
        let mut info = HashMap::new();
        info.insert(
            "uuid".to_string(),
            OwnedValue::try_from(Value::Str(uuid.into())).unwrap(),
        );
        info.insert(
            "state".to_string(),
            OwnedValue::try_from(Value::F64(state)).unwrap(),
        );
        info
    }
}

/// Integration test for GNOME extension startup retry logic.
///
/// Simulates the real-world scenario where:
/// 1. Service starts early, extension is in INITIALIZED state (6)
/// 2. After ~500ms, GNOME Shell finishes loading and state becomes ENABLED (1)
/// 3. The daemon's retry logic should detect the transition
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_gnome_extension_delayed_activation() {
    with_test_timeout(async {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start().expect("Failed to start dbus-daemon");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Start with INITIALIZED state (6)
        let state = Arc::new(AtomicU64::new(f64::to_bits(6.0)));
        let state_clone = state.clone();

        let mock_service = MockGnomeShellExtensionsDelayed {
            state: state.clone(),
        };

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to set bus name")
            .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
            .expect("Failed to serve mock service")
            .build()
            .await
            .expect("Failed to build connection");

        // Wait for service registration
        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .unwrap();
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has| has)
            }
        })
        .await
        .expect("Timeout waiting for mock service");

        // Spawn task to change state to ENABLED after 500ms
        let delay_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            state_clone.store(f64::to_bits(1.0), Ordering::SeqCst);
        });

        // Create blocking connection for probing
        let client_connection = zbus::blocking::connection::Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .expect("Failed to connect client");

        // Simulate retry logic: poll every 50ms until active or timeout
        let start = Instant::now();
        let mut status = match gnome_extension_dbus_probe_with_connection(&client_connection) {
            GnomeDbusProbeResult::Status(status) => status,
            GnomeDbusProbeResult::ShellUnavailable => {
                panic!("Initial probe unexpectedly reported shell unavailable")
            }
            GnomeDbusProbeResult::ProbeFailed => panic!("Initial probe unexpectedly failed"),
        };

        assert!(!status.active, "Initial state should not be active");
        assert_eq!(
            status.state,
            Some(6),
            "Initial state should be INITIALIZED (6)"
        );

        while !status.active && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
            status = match gnome_extension_dbus_probe_with_connection(&client_connection) {
                GnomeDbusProbeResult::Status(status) => status,
                GnomeDbusProbeResult::ShellUnavailable => {
                    panic!("Probe unexpectedly reported shell unavailable")
                }
                GnomeDbusProbeResult::ProbeFailed => panic!("Probe unexpectedly failed"),
            };
        }

        let elapsed = start.elapsed();

        // Verify success
        assert!(status.active, "Extension should become active after delay");
        assert_eq!(status.state, Some(1), "Final state should be ENABLED (1)");

        // Verify timing: should take ~500ms (allow 450-800ms for CI variance)
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed <= Duration::from_millis(800),
            "Expected ~500ms delay, got {:?}",
            elapsed
        );

        delay_task.await.unwrap();
    })
    .await;
}
