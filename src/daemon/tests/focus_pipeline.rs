use super::*;

#[tokio::test]
async fn test_update_status_for_focus_updates_snapshot() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    let win = win("firefox", "");
    let actions =
        update_status_for_focus(&handler, &status_broadcaster, &win, &kanata, "default").await;
    assert!(actions.is_some());

    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "browser");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_update_status_for_focus_unknown_layer_uses_default() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new(
        "127.0.0.1",
        10000,
        Some("default".to_string()),
        true,
        status_broadcaster.clone(),
    );

    {
        let mut inner = kanata.inner.try_lock().expect("Expected KanataClient lock");
        inner.known_layers = vec!["default".to_string()];
    }

    let win = win("firefox", "");
    let actions =
        update_status_for_focus(&handler, &status_broadcaster, &win, &kanata, "default").await;
    assert!(actions.is_some());

    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "default");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_handle_focus_event_ignored_when_paused_no_status_change() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    pause_broadcaster.set_paused(true);
    let win = win("firefox", "");
    let actions = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &win,
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_none());
    let snapshot = status_broadcaster.snapshot();
    assert!(snapshot.layer.is_empty());

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
    assert!(actions.is_some());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "browser");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_handle_focus_event_unfocus_paused_does_not_switch_layer() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    status_broadcaster.update_layer("current".to_string(), LayerSource::External);
    pause_broadcaster.set_paused(true);

    let actions = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &WindowInfo::default(),
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_none());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "current");
    assert_eq!(snapshot.layer_source, LayerSource::External);
}

#[tokio::test]
async fn test_update_status_for_focus_filters_invalid_virtual_keys() {
    // Rule with a virtual key that's NOT in the known list
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("invalid_vk".to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new(
        "127.0.0.1",
        10000,
        Some("default".to_string()),
        true,
        status_broadcaster.clone(),
    );

    {
        let mut inner = kanata.inner.try_lock().expect("Expected KanataClient lock");
        // Set known VKs to a list that does NOT include invalid_vk
        inner.known_virtual_keys = Some(vec!["valid_vk".to_string()]);
    }

    let win = win("firefox", "");
    let _actions =
        update_status_for_focus(&handler, &status_broadcaster, &win, &kanata, "default").await;

    let snapshot = status_broadcaster.snapshot();
    // The invalid VK should be filtered out - status should show NO virtual keys
    assert!(
        snapshot.virtual_keys.is_empty(),
        "Invalid VK should not appear in status snapshot, got: {:?}",
        snapshot.virtual_keys
    );
}

#[tokio::test]
async fn test_update_status_for_focus_shows_valid_virtual_keys() {
    // Rule with a virtual key that IS in the known list
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new(
        "127.0.0.1",
        10000,
        Some("default".to_string()),
        true,
        status_broadcaster.clone(),
    );

    {
        let mut inner = kanata.inner.try_lock().expect("Expected KanataClient lock");
        // Set known VKs to include vk_browser
        inner.known_virtual_keys = Some(vec!["vk_browser".to_string()]);
    }

    let win = win("firefox", "");
    let _actions =
        update_status_for_focus(&handler, &status_broadcaster, &win, &kanata, "default").await;

    let snapshot = status_broadcaster.snapshot();
    // The valid VK should appear in the status
    assert_eq!(
        snapshot.virtual_keys,
        vec!["vk_browser"],
        "Valid VK should appear in status snapshot"
    );
}
