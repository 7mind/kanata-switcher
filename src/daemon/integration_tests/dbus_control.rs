use super::*;

// === DBus Integration Tests ===

/// Test that the DBus service correctly processes WindowFocus calls and sends layer changes
#[tokio::test]
async fn test_dbus_service_layer_switching() {
    with_test_timeout(async {
        // Start mock kanata server
        let server = MockKanataServer::start();

        // Create rules
        let rules = vec![
            Rule {
                class: Some("firefox".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("terminal".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        // Create kanata client and connect
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        // Skip handshake messages (RequestLayerNames, RequestFakeKeyNames)
        drain_kanata_messages(&server, Duration::from_millis(100));

        // Create the DBus service handler directly (without actual DBus)
        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Simulate WindowFocus call for firefox
        {
            let win = WindowInfo {
                class: "firefox".to_string(),
                title: "GitHub".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Verify layer change was sent
        let msg = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        // Simulate WindowFocus call for kitty
        {
            let win = WindowInfo {
                class: "kitty".to_string(),
                title: "bash".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Verify layer change was sent
        let msg = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            })
        );
    })
    .await;
}

/// Test DBus service with virtual key actions
#[tokio::test]
async fn test_dbus_service_virtual_keys() {
    with_test_timeout(async {
        let server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        // Skip handshake messages (RequestLayerNames, RequestFakeKeyNames)
        drain_kanata_messages(&server, Duration::from_millis(100));

        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Focus firefox
        {
            let win = WindowInfo {
                class: "firefox".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive layer change and VK press
        let msg1 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg1,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        let msg2 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg2,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );

        // Unfocus (empty window)
        {
            let win = WindowInfo {
                class: "".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive VK release and layer change to default
        let msg3 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg3,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );

        let msg4 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg4,
            Some(KanataMessage::ChangeLayer {
                new: "default".to_string()
            })
        );
    })
    .await;
}

/// Test DBus service with fallthrough rules
#[tokio::test]
async fn test_dbus_service_fallthrough() {
    with_test_timeout(async {
        let server = MockKanataServer::start_with_config(MockKanataConfig {
            virtual_keys: Some(vec![
                "vk_browser".to_string(),
                "vk_terminal".to_string(),
                "vk_vim".to_string(),
                "vk_notify".to_string(), // Used in this test's fallthrough rule
            ]),
        });

        // Use layers from mock server's known_layers: ["default", "browser", "terminal", "vim"]
        let rules = vec![
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: None,
                raw_vk_action: Some(vec![("vk_notify".to_string(), "Tap".to_string())]),
                fallthrough: true,
            },
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("terminal".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        // Skip handshake messages (RequestLayerNames, RequestFakeKeyNames)
        drain_kanata_messages(&server, Duration::from_millis(100));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        {
            let win = WindowInfo {
                class: "kitty".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive: browser layer, raw_vk tap, terminal layer
        let msg1 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg1,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        let msg2 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg2,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_notify".to_string(),
                action: "Tap".to_string(),
            })
        );

        let msg3 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg3,
            Some(KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            })
        );
    })
    .await;
}
