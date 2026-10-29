use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// === Windows Focus Pipeline Integration Tests ===
//
// These tests verify the focus pipeline using Windows-style process names as window class
// identifiers (e.g. "firefox", "notepad"), matching how WindowsBackend reports focus via
// normalize_process_name applied to QueryFullProcessImageNameW output.

fn make_window(process_name: &str) -> WindowInfo {
    WindowInfo {
        class: process_name.to_string(),
        title: String::new(),
        is_native_terminal: false,
    }
}

fn make_window_with_title(process_name: &str, title: &str) -> WindowInfo {
    WindowInfo {
        class: process_name.to_string(),
        title: title.to_string(),
        is_native_terminal: false,
    }
}

/// FocusHandler correctly matches a rule by process name and triggers a layer change.
/// Equivalent to test_x11_focus_handler_integration.
#[test]
fn test_windows_process_name_rule_matching() {
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
            class: Some("notepad".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("editor".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&make_window("firefox"), "default");
    assert!(actions.is_some());
    assert!(
        actions
            .unwrap()
            .actions
            .contains(&FocusAction::ChangeLayer("browser".to_string()))
    );

    let actions = handler.handle(&make_window("notepad"), "default");
    assert!(actions.is_some());
    assert!(
        actions
            .unwrap()
            .actions
            .contains(&FocusAction::ChangeLayer("editor".to_string()))
    );
}

/// Multiple sequential focus changes trigger the correct layer at each step.
/// Equivalent to test_x11_multiple_focus_changes.
#[test]
fn test_windows_multiple_focus_changes() {
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
            class: Some("notepad".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("editor".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus Firefox → browser layer
    let actions = handler.handle(&make_window("firefox"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("browser".to_string())));

    // Focus Notepad → editor layer
    let actions = handler.handle(&make_window("notepad"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("editor".to_string())));

    // Focus unmatched process → default layer
    let actions = handler.handle(&make_window("explorer"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("default".to_string())));

    // Refocus Firefox → browser layer again
    let actions = handler.handle(&make_window("firefox"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("browser".to_string())));
}

/// Unmatched process name falls back to the default layer.
#[test]
fn test_windows_no_match_falls_back_to_default() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    // Prime: match firefox so next call is a change
    handler.handle(&make_window("firefox"), "default");

    // Unmapped process triggers default layer
    let actions = handler.handle(&make_window("cmd"), "default").unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("default".to_string())));
}

/// Process name normalization (lowercase, no extension) works through the pipeline.
/// Windows backend produces names like "firefox" (not "Firefox.exe").
#[test]
fn test_windows_normalized_process_names() {
    let rules = vec![
        Rule {
            class: Some("my_app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("myapp".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // WindowsBackend produces "my_app" from "C:\Program Files\My App\My App.exe"
    let actions = handler.handle(&make_window("my_app"), "default");
    assert!(actions.is_some());
    assert!(
        actions
            .unwrap()
            .actions
            .contains(&FocusAction::ChangeLayer("myapp".to_string()))
    );
}

/// Title matching works alongside process name class matching.
#[test]
fn test_windows_title_matching() {
    let rules = vec![
        Rule {
            class: Some("windowsterminal".to_string()),
            title: Some("vim".to_string()),
            on_native_terminal: None,
            layer: Some("vim".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
        Rule {
            class: Some("windowsterminal".to_string()),
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
        .handle(&make_window_with_title("windowsterminal", "vim - main.rs"), "default")
        .unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("vim".to_string())));

    // Terminal without vim → terminal layer
    let actions = handler
        .handle(&make_window_with_title("windowsterminal", "PowerShell"), "default")
        .unwrap();
    assert!(actions.actions.contains(&FocusAction::ChangeLayer("terminal".to_string())));
}

/// Full async pipeline: WindowInfo → FocusHandler → KanataClient → mock server.
/// Equivalent to test_x11_focus_query_on_start_and_unpause (pipeline portion).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_windows_focus_pipeline_with_mock_kanata() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
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

        let firefox = make_window("firefox");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &firefox,
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

        // Switch to unmatched process → default layer
        let notepad = make_window("notepad");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &notepad,
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

/// Full async pipeline with pause/unpause cycle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_windows_pipeline_pause_unpause() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
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

        let firefox = make_window("firefox");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &firefox,
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

        pause_broadcaster.set_paused(true);
        drain_kanata_messages(&mock_server, Duration::from_millis(100));
        pause_broadcaster.set_paused(false);

        // After unpause, switch to a new app
        let cmd = make_window("cmd");
        let default_layer = kanata.default_layer_sync();
        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &cmd,
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
