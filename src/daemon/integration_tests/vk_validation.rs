use super::*;

// === Virtual Key Validation Tests ===

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_virtual_key_validation_valid_key() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true, // quiet mode to suppress output
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Drain handshake messages
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Send valid virtual key action (vk_browser is in default MockKanataConfig)
        let result = kanata.act_on_fake_key("vk_browser", "Press").await;
        assert!(result, "Valid virtual key should succeed");

        // Verify the message was sent to kanata
        let msg = mock_server.recv_timeout(Duration::from_secs(1));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string()
            })
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_virtual_key_validation_invalid_key() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true, // quiet mode to suppress output
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Drain handshake messages
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Send invalid virtual key action (nonexistent_vk is not in default MockKanataConfig)
        let result = kanata.act_on_fake_key("nonexistent_vk", "Press").await;
        assert!(!result, "Invalid virtual key should fail validation");

        // Verify no message was sent to kanata
        let msg = mock_server.recv_timeout(Duration::from_millis(100));
        assert!(
            msg.is_none(),
            "No message should be sent for invalid virtual key"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_virtual_key_validation_legacy_kanata() {
    with_test_timeout(async {
        // Start mock server simulating older kanata that doesn't support RequestFakeKeyNames
        let mock_server = MockKanataServer::start_legacy();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true,
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Drain handshake messages
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // With legacy kanata, any virtual key should be allowed (validation disabled)
        let result = kanata.act_on_fake_key("any_vk_name", "Press").await;
        assert!(
            result,
            "Legacy kanata should allow any virtual key (validation disabled)"
        );

        // Verify the message was sent to kanata
        let msg = mock_server.recv_timeout(Duration::from_secs(1));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "any_vk_name".to_string(),
                action: "Press".to_string()
            })
        );
    })
    .await;
}

/// Test that legacy kanata detection works: first connect triggers RequestFakeKeyNames,
/// error response causes reconnect, and on reconnect RequestFakeKeyNames is NOT sent again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_legacy_kanata_skips_request_fake_key_names_on_reconnect() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start_legacy();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true,
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Wait for reconnect (first connect fails with legacy detection, then reconnects after 1s)
        // Collect messages over 2 seconds to catch both first connect and reconnect
        let start = std::time::Instant::now();
        let mut all_messages = Vec::new();
        while start.elapsed() < Duration::from_secs(2) {
            if let Some(msg) = mock_server.recv_timeout(Duration::from_millis(100)) {
                all_messages.push(msg);
            }
        }

        // Verify RequestFakeKeyNames was sent on first attempt
        assert!(
            all_messages.contains(&KanataMessage::RequestFakeKeyNames),
            "First connection should have sent RequestFakeKeyNames, got: {:?}",
            all_messages
        );

        // Count how many times RequestLayerNames appears (once per connection)
        let layer_name_requests = all_messages
            .iter()
            .filter(|m| **m == KanataMessage::RequestLayerNames)
            .count();

        // Count how many times RequestFakeKeyNames appears (should be 1, only on first connect)
        let fake_key_requests = all_messages
            .iter()
            .filter(|m| **m == KanataMessage::RequestFakeKeyNames)
            .count();

        assert!(
            layer_name_requests >= 2,
            "Should have at least 2 RequestLayerNames (first connect + reconnect), got {}. Messages: {:?}",
            layer_name_requests,
            all_messages
        );
        assert_eq!(
            fake_key_requests, 1,
            "Should have exactly 1 RequestFakeKeyNames (only first connect), got {}",
            fake_key_requests
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_virtual_key_validation_empty_list_rejects_all() {
    with_test_timeout(async {
        // Start mock server with empty virtual keys list (newer kanata with no VKs defined)
        let mock_server = MockKanataServer::start_with_config(MockKanataConfig {
            virtual_keys: Some(vec![]),
        });
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true,
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Drain handshake messages
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // With empty list from newer kanata, ALL virtual keys should be rejected
        // (unlike legacy kanata which allows all because we don't know what exists)
        let result = kanata.act_on_fake_key("any_vk_name", "Press").await;
        assert!(
            !result,
            "Empty VK list from newer kanata should reject all virtual keys"
        );

        // Verify no message was sent to kanata
        let msg = mock_server.recv_timeout(Duration::from_millis(100));
        assert!(
            msg.is_none(),
            "No message should be sent when VK is rejected"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_virtual_key_validation_all_actions() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true,
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Drain handshake messages
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Test all action types with valid key
        for action in ["Press", "Release", "Tap", "Toggle"] {
            let result = kanata.act_on_fake_key("vk_vim", action).await;
            assert!(result, "Valid VK with action {} should succeed", action);

            let msg = mock_server.recv_timeout(Duration::from_secs(1));
            assert_eq!(
                msg,
                Some(KanataMessage::ActOnFakeKey {
                    name: "vk_vim".to_string(),
                    action: action.to_string()
                })
            );
        }

        // Test all action types with invalid key
        for action in ["Press", "Release", "Tap", "Toggle"] {
            let result = kanata.act_on_fake_key("invalid_vk", action).await;
            assert!(
                !result,
                "Invalid VK with action {} should fail validation",
                action
            );
        }

        // Verify no messages were sent for invalid keys
        let msg = mock_server.recv_timeout(Duration::from_millis(100));
        assert!(msg.is_none(), "No messages should be sent for invalid VKs");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_handshake_requests_fake_key_names() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            None,
            true,
            status_broadcaster,
        );

        kanata.connect_with_retry().await;

        // Collect all handshake messages
        let mut messages = Vec::new();
        while let Some(msg) = mock_server.recv_timeout(Duration::from_millis(100)) {
            messages.push(msg);
        }

        // Verify both RequestLayerNames and RequestFakeKeyNames were sent
        assert!(
            messages.contains(&KanataMessage::RequestLayerNames),
            "Handshake should include RequestLayerNames"
        );
        assert!(
            messages.contains(&KanataMessage::RequestFakeKeyNames),
            "Handshake should include RequestFakeKeyNames"
        );
    })
    .await;
}

/// Test that layer changes still happen even when VK in same rule is invalid
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_vk_does_not_block_layer_change() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Rule with layer change AND invalid VK
        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("invalid_vk".to_string()), // Not in mock server's VK list
            raw_vk_action: None,
            fallthrough: false,
        }];

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Trigger focus
        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test".to_string(),
            is_native_terminal: false,
        };
        let default_layer = kanata.default_layer().await.unwrap_or_default();
        let actions = handler.lock().unwrap().handle(&win, &default_layer);
        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }

        // Layer change should still happen
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            }),
            "Layer change should happen even with invalid VK"
        );

        // No VK message should be sent (invalid VK was skipped)
        let msg = mock_server.recv_timeout(Duration::from_millis(100));
        assert!(
            msg.is_none(),
            "Invalid VK action should be skipped silently"
        );
    })
    .await;
}

/// Test that layer changes still happen on legacy kanata (no VK validation) when VK in same rule
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_vk_does_not_block_layer_change_legacy_kanata() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start_legacy();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        // Wait for reconnect after legacy detection
        tokio::time::sleep(Duration::from_secs(2)).await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Rule with layer change AND VK (which won't be validated on legacy kanata)
        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("any_vk".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Use the full flow through update_status_for_focus (like production)
        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test".to_string(),
            is_native_terminal: false,
        };
        let default_layer = kanata.default_layer().await.unwrap_or_default();
        let actions =
            update_status_for_focus(&handler, &status_broadcaster, &win, &kanata, &default_layer)
                .await;

        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }

        // Layer change should still happen
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            }),
            "Layer change should happen on legacy kanata"
        );
    })
    .await;
}

/// Test that valid VKs work even when other VKs in different rules are invalid
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_valid_vk_works_with_invalid_vk_in_other_rule() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Two rules: one with valid VK, one with invalid VK (fallthrough)
        let rules = vec![
            Rule {
                class: Some("test-app".to_string()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: Some("invalid_vk".to_string()), // Invalid
                raw_vk_action: None,
                fallthrough: true, // Continue to next rule
            },
            Rule {
                class: Some("test-app".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: Some("vk_browser".to_string()), // Valid (in mock server list)
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test".to_string(),
            is_native_terminal: false,
        };
        let default_layer = kanata.default_layer().await.unwrap_or_default();
        let actions = handler.lock().unwrap().handle(&win, &default_layer);
        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }

        // Collect all messages
        let mut messages = Vec::new();
        while let Some(msg) = mock_server.recv_timeout(Duration::from_millis(200)) {
            messages.push(msg);
        }

        // Should have layer change and valid VK press (invalid VK skipped)
        assert!(
            messages.contains(&KanataMessage::ChangeLayer {
                new: "browser".to_string()
            }),
            "Layer change should happen"
        );
        assert!(
            messages.contains(&KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string()
            }),
            "Valid VK should be pressed"
        );
        // Invalid VK should NOT be in messages
        assert!(
            !messages.iter().any(
                |m| matches!(m, KanataMessage::ActOnFakeKey { name, .. } if name == "invalid_vk")
            ),
            "Invalid VK should not be sent"
        );
    })
    .await;
}

/// Test raw_vk_action with mix of valid and invalid VKs
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raw_vk_action_mixed_valid_invalid() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Rule with multiple raw_vk_actions: some valid, some invalid
        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: Some(vec![
                ("vk_vim".to_string(), "Tap".to_string()),          // Valid
                ("invalid_vk1".to_string(), "Press".to_string()),   // Invalid
                ("vk_terminal".to_string(), "Toggle".to_string()),  // Valid
                ("invalid_vk2".to_string(), "Release".to_string()), // Invalid
            ]),
            fallthrough: false,
        }];

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test".to_string(),
            is_native_terminal: false,
        };
        let default_layer = kanata.default_layer().await.unwrap_or_default();
        let actions = handler.lock().unwrap().handle(&win, &default_layer);
        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }

        // Collect all messages
        let mut messages = Vec::new();
        while let Some(msg) = mock_server.recv_timeout(Duration::from_millis(200)) {
            messages.push(msg);
        }

        // Layer change should happen
        assert!(
            messages.contains(&KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            }),
            "Layer change should happen regardless of invalid VKs"
        );

        // Valid VKs should be sent
        assert!(
            messages.contains(&KanataMessage::ActOnFakeKey {
                name: "vk_vim".to_string(),
                action: "Tap".to_string()
            }),
            "Valid vk_vim Tap should be sent"
        );
        assert!(
            messages.contains(&KanataMessage::ActOnFakeKey {
                name: "vk_terminal".to_string(),
                action: "Toggle".to_string()
            }),
            "Valid vk_terminal Toggle should be sent"
        );

        // Invalid VKs should NOT be in messages
        assert!(
            !messages.iter().any(
                |m| matches!(m, KanataMessage::ActOnFakeKey { name, .. } if name == "invalid_vk1")
            ),
            "Invalid vk1 should not be sent"
        );
        assert!(
            !messages.iter().any(
                |m| matches!(m, KanataMessage::ActOnFakeKey { name, .. } if name == "invalid_vk2")
            ),
            "Invalid vk2 should not be sent"
        );
    })
    .await;
}

/// Test that releasing an invalid VK doesn't affect other releases
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_invalid_vk_release_does_not_block_valid_releases() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // First rule: press valid VK
        // Second rule: press invalid VK (via fallthrough)
        let rules = vec![
            Rule {
                class: Some("app1".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: Some("vk_browser".to_string()), // Valid
                raw_vk_action: None,
                fallthrough: false,
            },
            Rule {
                class: Some("app2".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("terminal".to_string()),
                virtual_key: Some("vk_terminal".to_string()), // Valid
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Focus app1 - should press vk_browser
        {
            let win = WindowInfo {
                class: "app1".to_string(),
                title: "Test".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        // Focus app2 - should release vk_browser and press vk_terminal
        {
            let win = WindowInfo {
                class: "app2".to_string(),
                title: "Test".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Collect messages from the switch
        let mut messages = Vec::new();
        while let Some(msg) = mock_server.recv_timeout(Duration::from_millis(200)) {
            messages.push(msg);
        }

        // Should have: release vk_browser, change layer, press vk_terminal
        assert!(
            messages.contains(&KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string()
            }),
            "vk_browser should be released"
        );
        assert!(
            messages.contains(&KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            }),
            "Layer should change to terminal"
        );
        assert!(
            messages.contains(&KanataMessage::ActOnFakeKey {
                name: "vk_terminal".to_string(),
                action: "Press".to_string()
            }),
            "vk_terminal should be pressed"
        );
    })
    .await;
}
