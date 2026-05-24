use super::*;

/// Test with a private DBus session (no dependency on desktop session)
///
/// This test verifies the DBus transport layer works correctly by:
/// 1. Starting a private dbus-daemon
/// 2. Registering the service on that bus
/// 3. Calling the service method from a client connection
/// 4. Verifying the layer change reaches the mock Kanata server
///
/// Requires dbus-daemon to be available. Skips gracefully if not found.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_service_real_bus() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        // Start private dbus-daemon
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();
        let port = mock_server.port();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()), // must be in mock server's known_layers
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        // Parse the bus address
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        // Create the kanata client and connect
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            port,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        // Skip RequestLayerNames
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Connect to bus and register service
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register service");

        // Create client connection
        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        // Wait for service name to be registered on bus
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        // Keep service connection alive by holding a reference
        let _service_conn = service_connection;

        // Call WindowFocus method
        let result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;

        // Check if the call succeeded
        assert!(result.is_ok(), "DBus call failed: {:?}", result.err());

        // Verify layer change (recv_timeout handles waiting)
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
    })
    .await;
}

/// Test that GetStatus reports the initial layer without waiting for a layer change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_get_status_initial_layer() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let reply = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "GetStatus",
                &(),
            )
            .await
            .expect("GetStatus call failed");

        let (layer, virtual_keys, source): (String, Vec<String>, String) = reply
            .body()
            .deserialize()
            .expect("Failed to deserialize GetStatus response");

        assert_eq!(layer, "default");
        assert!(virtual_keys.is_empty());
        assert_eq!(source, "external");
    })
    .await;
}

/// Test that focus-based status updates override the layer source on GetStatus.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_get_status_focus_source() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let focus_result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );

        let reply = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "GetStatus",
                &(),
            )
            .await
            .expect("GetStatus call failed");

        let (layer, _virtual_keys, source): (String, Vec<String>, String) = reply
            .body()
            .deserialize()
            .expect("Failed to deserialize GetStatus response");

        assert_eq!(layer, "browser");
        assert_eq!(source, "focus");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_paused_changed_signal() {
    with_test_timeout(async {
        use futures_util::StreamExt;
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let proxy = zbus::Proxy::new(
            &client,
            TEST_DAEMON_DBUS_NAME,
            "/com/github/kanata/Switcher",
            "com.github.kanata.Switcher",
        )
        .await
        .expect("Failed to create proxy");
        let mut paused_stream = proxy
            .receive_signal("PausedChanged")
            .await
            .expect("Failed to subscribe to PausedChanged");

        let pause_result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Pause",
                &(),
            )
            .await;
        assert!(
            pause_result.is_ok(),
            "DBus Pause failed: {:?}",
            pause_result.err()
        );

        let paused_msg = tokio::time::timeout(Duration::from_secs(2), paused_stream.next())
            .await
            .expect("PausedChanged signal timed out")
            .expect("PausedChanged stream closed");
        let paused: bool = paused_msg
            .body()
            .deserialize()
            .expect("Failed to deserialize PausedChanged");
        assert!(paused, "Expected paused=true");

        let unpause_result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Unpause",
                &(),
            )
            .await;
        assert!(
            unpause_result.is_ok(),
            "DBus Unpause failed: {:?}",
            unpause_result.err()
        );

        let unpaused_msg = tokio::time::timeout(Duration::from_secs(2), paused_stream.next())
            .await
            .expect("PausedChanged signal timed out")
            .expect("PausedChanged stream closed");
        let paused: bool = unpaused_msg
            .body()
            .deserialize()
            .expect("Failed to deserialize PausedChanged");
        assert!(!paused, "Expected paused=false");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_status_changed_focus_signal() {
    with_test_timeout(async {
        use futures_util::StreamExt;
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let proxy = zbus::Proxy::new(
            &client,
            TEST_DAEMON_DBUS_NAME,
            "/com/github/kanata/Switcher",
            "com.github.kanata.Switcher",
        )
        .await
        .expect("Failed to create proxy");
        let mut status_stream = proxy
            .receive_signal("StatusChanged")
            .await
            .expect("Failed to subscribe to StatusChanged");

        let focus_result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );

        let mut focus_signal: Option<(String, Vec<String>, String)> = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let msg = tokio::time::timeout(Duration::from_secs(2), status_stream.next())
                .await
                .ok()
                .flatten();
            if let Some(message) = msg {
                let (layer, virtual_keys, source): (String, Vec<String>, String) = message
                    .body()
                    .deserialize()
                    .expect("Failed to deserialize StatusChanged");
                if source == "focus" {
                    focus_signal = Some((layer, virtual_keys, source));
                    break;
                }
            } else {
                break;
            }
        }

        let (layer, _virtual_keys, source) =
            focus_signal.expect("Expected a StatusChanged signal with focus source");
        assert_eq!(layer, "browser");
        assert_eq!(source, "focus");
    })
    .await;
}
