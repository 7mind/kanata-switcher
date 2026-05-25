use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// === macOS Focus Pipeline Integration Tests ===
//
// These tests verify the focus pipeline using macOS-style bundle IDs as window class
// identifiers (e.g. "com.apple.Safari"), matching how MacOsBackend reports focus.
// They are the macOS equivalent of the X11/Wayland integration tests.

fn make_window(bundle_id: &str) -> WindowInfo {
    WindowInfo {
        class: bundle_id.to_string(),
        title: String::new(),
        is_native_terminal: false,
    }
}

fn make_window_with_title(bundle_id: &str, title: &str) -> WindowInfo {
    WindowInfo {
        class: bundle_id.to_string(),
        title: title.to_string(),
        is_native_terminal: false,
    }
}

/// FocusHandler correctly matches a rule by bundle ID and triggers a layer change.
/// Equivalent to test_x11_focus_handler_integration.
#[test]
fn test_macos_bundle_id_rule_matching() {
    let rules = vec![
        Rule {
            class: Some("com.apple.Safari".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
        Rule {
            class: Some("com.apple.Terminal".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&make_window("com.apple.Safari"), "default");
    assert!(actions.is_some());
    assert!(
        actions
            .unwrap()
            .actions
            .contains(&FocusAction::ChangeLayer("browser".to_string()))
    );

    let actions = handler.handle(&make_window("com.apple.Terminal"), "default");
    assert!(actions.is_some());
    assert!(
        actions
            .unwrap()
            .actions
            .contains(&FocusAction::ChangeLayer("terminal".to_string()))
    );
}

/// Multiple sequential focus changes trigger the correct layer at each step.
/// Equivalent to test_x11_multiple_focus_changes.
#[test]
fn test_macos_multiple_focus_changes() {
    let rules = vec![
        Rule {
            class: Some("com.apple.Safari".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
        Rule {
            class: Some("com.apple.finder".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("files".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus Safari → browser layer
    let actions = handler.handle(&make_window("com.apple.Safari"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("browser".to_string())));

    // Focus Finder → files layer
    let actions = handler.handle(&make_window("com.apple.finder"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("files".to_string())));

    // Focus unmatched app → default layer
    let actions = handler.handle(&make_window("com.microsoft.VSCode"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("default".to_string())));

    // Refocus Safari → browser layer again
    let actions = handler.handle(&make_window("com.apple.Safari"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("browser".to_string())));
}

/// Unmatched bundle ID falls back to the default layer.
#[test]
fn test_macos_no_match_falls_back_to_default() {
    let rules = vec![Rule {
        class: Some("com.apple.Safari".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    // Prime: match Safari so next call is a change
    handler.handle(&make_window("com.apple.Safari"), "default");

    // Unmapped app triggers default layer
    let actions = handler.handle(&make_window("com.microsoft.VSCode"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("default".to_string())));
}

/// Regex patterns work correctly with bundle ID dot separators.
#[test]
fn test_macos_regex_bundle_id_pattern() {
    let rules = vec![Rule {
        class: Some(r"^com\.apple\..*".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("apple".to_string()),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }];

    for bundle_id in &["com.apple.Safari", "com.apple.finder", "com.apple.Terminal"] {
        let mut handler = FocusHandler::new(rules.clone(), None, true);
        let actions = handler.handle(&make_window(bundle_id), "default");
        assert!(actions.is_some(), "Expected match for {}", bundle_id);
        assert!(
            actions
                .unwrap()
                .actions
                .contains(&FocusAction::ChangeLayer("apple".to_string())),
            "Expected apple layer for {}",
            bundle_id
        );
    }
}

/// Title matching works alongside bundle ID class matching on macOS.
#[test]
fn test_macos_title_matching() {
    let rules = vec![
        Rule {
            class: Some("com.apple.Terminal".to_string()),
            title: Some("vim".to_string()),
            on_native_terminal: None,
            layer: Some("vim".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
        Rule {
            class: Some("com.apple.Terminal".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Terminal with vim in title → vim layer
    let actions = handler
        .handle(&make_window_with_title("com.apple.Terminal", "vim - main.rs"), "default")
        .unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("vim".to_string())));

    // Terminal without vim → terminal layer
    let actions = handler
        .handle(&make_window_with_title("com.apple.Terminal", "bash"), "default")
        .unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("terminal".to_string())));
}

/// Full async pipeline: WindowInfo → FocusHandler → KanataClient → mock server.
/// Equivalent to test_x11_focus_query_on_start_and_unpause (pipeline portion).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_macos_focus_pipeline_with_mock_kanata() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("com.apple.Safari".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
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

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let safari = make_window("com.apple.Safari");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &safari,
            &kanata,
            &default_layer,
        )
        .await
        {
            execute_focus_actions(&kanata, actions).await;
        }

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer { new: "browser".to_string() },
            Duration::from_secs(2),
        );

        // Switch to unmatched app → default layer
        let other = make_window("com.microsoft.VSCode");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &other,
            &kanata,
            &default_layer,
        )
        .await
        {
            execute_focus_actions(&kanata, actions).await;
        }

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer { new: "default".to_string() },
            Duration::from_secs(2),
        );
    })
    .await;
}

/// Full async pipeline with multiple focus changes and pause/unpause cycle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_macos_pipeline_pause_unpause() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("com.apple.Safari".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
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

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let safari = make_window("com.apple.Safari");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &safari,
            &kanata,
            &default_layer,
        )
        .await
        {
            execute_focus_actions(&kanata, actions).await;
        }
        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer { new: "browser".to_string() },
            Duration::from_secs(2),
        );

        // While paused, events are still processed but pause state reflects the pause
        pause_broadcaster.set_paused(true);
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        pause_broadcaster.set_paused(false);

        // After unpause, send another event
        let finder = make_window("com.apple.finder");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &finder,
            &kanata,
            &default_layer,
        )
        .await
        {
            execute_focus_actions(&kanata, actions).await;
        }
        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer { new: "default".to_string() },
            Duration::from_secs(2),
        );
    })
    .await;
}

/// Verifies current_window_info() returns a valid (non-panicking) WindowInfo on macOS.
/// On headless CI, NSWorkspace.frontmostApplication may be nil, returning an empty bundle ID.
#[test]
fn test_macos_current_window_info_does_not_panic() {
    let info = current_window_info();
    assert!(!info.is_native_terminal, "macOS apps are never native terminals");
    // class is either empty (no frontmost app in headless env) or a valid bundle ID
}
