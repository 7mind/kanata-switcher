use super::*;

/// Test pause/unpause flow with a mock Kanata server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_pause_unpause() {
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
            virtual_key: Some("vk_browser".to_string()),
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

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );

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

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        if let Some(message) = msg {
            assert_eq!(
                message,
                KanataMessage::ChangeLayer {
                    new: "default".to_string()
                }
            );
        }

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
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");

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

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(msg, Some(KanataMessage::RequestLayerNames));

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(msg, Some(KanataMessage::RequestFakeKeyNames));

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_handle_focus_event_ignored_when_paused() {
    with_test_timeout(async {
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

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        pause_broadcaster.set_paused(true);
        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test Window".to_string(),
            is_native_terminal: false,
        };
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &win,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_none(), "Expected no actions while paused");
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");

        pause_broadcaster.set_paused(false);
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &win,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_some(), "Expected actions after unpause");
        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_pause_wayland_env() {
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

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let pause_state = pause_broadcaster.clone();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
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
        let _dbus_service_guard = register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Wayland,
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
        assert!(pause_state.is_paused(), "Expected daemon to be paused");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_unfocus_ignored_when_paused() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        pause_broadcaster.set_paused(true);
        let unfocus = WindowInfo::default();
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &unfocus,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_none(), "Expected no actions while paused");
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_pause_daemon_releases_virtual_keys_and_resets_layer() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        {
            let win = WindowInfo {
                class: "test-app".to_string(),
                title: "Test Window".to_string(),
                is_native_terminal: false,
            };
            let actions = handler.lock().unwrap().handle(&win, "default");
            assert!(actions.is_some());
        }

        let pause_broadcaster = pause_broadcaster.clone();
        let handler = handler.clone();
        let status_broadcaster = status_broadcaster.clone();
        let kanata = kanata.clone();
        pause_daemon_direct(
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        if let Some(message) = msg {
            assert_eq!(
                message,
                KanataMessage::ChangeLayer {
                    new: "default".to_string()
                }
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_pause_unpause_private_dbus() {
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
            virtual_key: Some("vk_browser".to_string()),
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
        let mut pause_receiver = pause_broadcaster.subscribe();
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
            pause_broadcaster.clone(),
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

        let pause_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Pause).await;
        assert!(
            pause_result.is_ok(),
            "Pause control command failed: {:?}",
            pause_result.err()
        );

        let pause_changed =
            tokio::time::timeout(Duration::from_secs(2), pause_receiver.changed()).await;
        assert!(pause_changed.is_ok(), "Pause broadcast timed out");
        assert!(*pause_receiver.borrow(), "Expected paused state true");

        let unpause_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Unpause).await;
        assert!(
            unpause_result.is_ok(),
            "Unpause control command failed: {:?}",
            unpause_result.err()
        );

        let unpause_changed =
            tokio::time::timeout(Duration::from_secs(2), pause_receiver.changed()).await;
        assert!(unpause_changed.is_ok(), "Unpause broadcast timed out");
        assert!(!*pause_receiver.borrow(), "Expected paused state false");
    })
    .await;
}
