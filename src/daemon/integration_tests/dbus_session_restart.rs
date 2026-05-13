use super::*;

/// Test that Restart requests trigger the restart channel.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_restart_request() {
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
        let mut restart_receiver = restart_handle.subscribe();
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

        let restart_result = client
            .call_method(
                Some(TEST_DAEMON_DBUS_NAME),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Restart",
                &(),
            )
            .await;
        assert!(
            restart_result.is_ok(),
            "DBus Restart failed: {:?}",
            restart_result.err()
        );

        let changed =
            tokio::time::timeout(Duration::from_secs(2), restart_receiver.changed()).await;
        assert!(changed.is_ok(), "Restart signal timed out");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_restart_private_dbus() {
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
        let mut restart_receiver = restart_handle.subscribe();
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

        let control_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Restart).await;
        assert!(
            control_result.is_ok(),
            "Restart control command failed: {:?}",
            control_result.err()
        );

        let changed =
            tokio::time::timeout(Duration::from_secs(2), restart_receiver.changed()).await;
        assert!(changed.is_ok(), "Restart signal timed out");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_returns_error_without_service() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");
        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let result = send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Restart).await;
        assert!(result.is_err(), "Expected error when service is missing");
    })
    .await;
}
