use super::*;

#[test]
fn test_basic_layer_match() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("browser".to_string())]
    );
}

#[test]
fn test_no_match_uses_default() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("default".to_string())]
    );
}

#[test]
fn test_same_window_no_action() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab1"), "default");
    assert_eq!(actions, None);
}

#[test]
fn test_same_rule_different_window_no_action() {
    let rules = vec![rule(None, None, Some("global"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", "tab1"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("global".to_string())]
    );

    let actions = handler.handle(&win("kitty", "tab2"), "default");
    assert_eq!(actions, None);
}

#[test]
fn test_same_rule_different_window_no_action_with_vk_and_raw() {
    let rules = vec![Rule {
        class: None,
        title: None,
        on_native_terminal: None,
        layer: Some("global".to_string()),
        virtual_key: Some("vk_global".to_string()),
        raw_vk_action: Some(vec![("vk_raw".to_string(), "Tap".to_string())]),
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", "tab1"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("global".to_string()),
            FocusAction::PressVk("vk_global".to_string()),
            FocusAction::RawVkAction("vk_raw".to_string(), "Tap".to_string()),
        ]
    );

    let actions = handler.handle(&win("kitty", "tab2"), "default");
    assert_eq!(actions, None);
}
#[test]
fn test_title_change_triggers_action() {
    let rules = vec![
        rule(Some("kitty"), Some("vim"), Some("vim")),
        rule(Some("kitty"), None, Some("terminal")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", "bash"), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["terminal".to_string()]);

    let actions = handler.handle(&win("kitty", "vim"), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["vim".to_string()]);
}

#[test]
fn test_unfocus_releases_vk_and_switches_to_default() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("", ""), "default").unwrap();

    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ReleaseVk("vk_browser".to_string()),
            FocusAction::ChangeLayer("default".to_string()),
        ]
    );
    assert_eq!(actions.new_managed_vks, Vec::<String>::new());
}

#[test]
fn test_native_terminal_rule_applies_actions() {
    let rules = vec![rule(Some("kitty"), None, Some("terminal"))];
    let native_rule = Some(NativeTerminalRule {
        layer: "tty".to_string(),
        virtual_key: Some("vk_tty".to_string()),
        raw_vk_action: vec![("vk_notify".to_string(), "Tap".to_string())],
    });
    let mut handler = FocusHandler::new(rules, native_rule, true);

    let actions = handler
        .handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: true,
            },
            "default",
        )
        .unwrap();

    assert!(has_action(
        &actions,
        &FocusAction::ChangeLayer("tty".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_tty".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::RawVkAction("vk_notify".to_string(), "Tap".to_string())
    ));
}

#[test]
fn test_native_terminal_without_rule_uses_default() {
    let rules = vec![rule(Some("kitty"), None, Some("terminal"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler
        .handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: true,
            },
            "default",
        )
        .unwrap();

    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("default".to_string())]
    );
}

#[test]
fn test_paused_status_resets_virtual_keys_and_source() {
    let status_broadcaster = StatusBroadcaster::new();
    status_broadcaster.update_layer("external".to_string(), LayerSource::External);
    status_broadcaster.update_virtual_keys(vec!["vk_browser".to_string()]);
    status_broadcaster.set_paused_status("base".to_string());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "base");
    assert!(snapshot.virtual_keys.is_empty());
    assert_eq!(snapshot.layer_source, LayerSource::External);
}

#[test]
fn test_virtual_key_press_on_focus() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_browser".to_string())
    ));
    assert!(
        !actions
            .actions
            .iter()
            .any(|a| matches!(a, FocusAction::ReleaseVk(_)))
    );
    assert_eq!(actions.new_managed_vks, vec!["vk_browser".to_string()]);
}

#[test]
fn test_virtual_key_release_on_switch() {
    let rules = vec![
        rule_vk(Some("firefox"), "vk_browser"),
        rule_vk(Some("kitty"), "vk_terminal"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Release comes before Press in the action list
    let release_idx = actions
        .actions
        .iter()
        .position(|a| matches!(a, FocusAction::ReleaseVk(_)));
    let press_idx = actions
        .actions
        .iter()
        .position(|a| matches!(a, FocusAction::PressVk(_)));
    assert!(release_idx.unwrap() < press_idx.unwrap());

    assert!(has_action(
        &actions,
        &FocusAction::ReleaseVk("vk_browser".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_terminal".to_string())
    ));
}

#[test]
fn test_virtual_key_no_change_no_press() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab2"), "default");

    // Window changed but VK is the same - no VK actions (VK already held)
    assert!(
        actions.is_none()
            || !actions
                .as_ref()
                .unwrap()
                .actions
                .iter()
                .any(|a| matches!(a, FocusAction::PressVk(_) | FocusAction::ReleaseVk(_)))
    );
}

#[test]
fn test_partial_vk_set_change_only_releases_removed() {
    // Two rules with fallthrough: vk1 and vk2 are both held
    // Then switch to a window that only matches vk2 - only vk1 should be released
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: Some("both".to_string()),
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus window that matches both rules - both VKs pressed
    let actions = handler.handle(&win("app", "both"), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string()]
    );

    // Focus window that only matches second rule - only vk1 should be released, vk2 stays held
    let actions = handler.handle(&win("app", "other"), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::ReleaseVk("vk1".to_string())
    ));
    assert!(!has_action(
        &actions,
        &FocusAction::ReleaseVk("vk2".to_string())
    ));
    assert!(!has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    )); // vk2 already held
    assert_eq!(actions.new_managed_vks, vec!["vk2".to_string()]);
}

#[test]
fn test_unfocus_releases_multiple_vks_in_reverse_order() {
    // Multiple VKs held should be released in reverse order (bottom-to-top)
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk3".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus window - all three VKs pressed in order
    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]
    );

    // Unfocus - all VKs should be released in reverse order (vk3, vk2, vk1)
    let actions = handler.handle(&win("", ""), "default").unwrap();
    let release_actions: Vec<_> = actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::ReleaseVk(vk) = a {
                Some(vk.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        release_actions,
        vec!["vk3".to_string(), "vk2".to_string(), "vk1".to_string()]
    );
}

#[test]
fn test_raw_vk_action_fires_on_focus() {
    let rules = vec![rule_raw_vk(
        Some("firefox"),
        vec![("vk1", "Tap"), ("vk2", "Toggle")],
    )];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("vk1".to_string(), "Tap".to_string()),
            ("vk2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_fallthrough_collects_all_layers() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("kitty"), None, Some("layer1"))),
        rule(Some("kitty"), None, Some("layer2")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both layers should be in the action list, in order
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer2".to_string()]
    );
}

#[test]
fn test_fallthrough_add_remove_rule_only_new_actions() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("base"))),
        rule(Some("app"), Some("special"), Some("special")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", "special"), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec!["base".to_string(), "special".to_string()]
    );

    let actions = handler.handle(&win("app", "other"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("base".to_string())]
    );
}

#[test]
fn test_fallthrough_collects_all_raw_vk_actions() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(Some("kitty"), vec![("vk1", "Press")])),
        rule_raw_vk(Some("kitty"), vec![("vk2", "Tap")]),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("vk1".to_string(), "Press".to_string()),
            ("vk2".to_string(), "Tap".to_string()),
        ]
    );
}

#[test]
fn test_fallthrough_all_vks_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_vk(Some("kitty"), "vk2"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both vk1 and vk2 should be pressed (all matched VKs are held)
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string()]
    );
}

#[test]
fn test_fallthrough_multiple_vks_all_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk2")),
        rule_vk(Some("kitty"), "vk3"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // All three VKs should be pressed and held
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk3".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]
    );
}

#[test]
fn test_fallthrough_action_order_preserved() {
    // Test that actions from each rule are in order: layer, vk, raw_vk
    let rules = vec![
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer1".to_string()),
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: Some(vec![("raw1".to_string(), "Tap".to_string())]),
            fallthrough: true,
        },
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer2".to_string()),
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: Some(vec![("raw2".to_string(), "Toggle".to_string())]),
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Expected order: layer1, PressVk(vk1), raw1, layer2, PressVk(vk2), raw2
    // All matched VKs are pressed (not tapped)
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("layer1".to_string()),
            FocusAction::PressVk("vk1".to_string()),
            FocusAction::RawVkAction("raw1".to_string(), "Tap".to_string()),
            FocusAction::ChangeLayer("layer2".to_string()),
            FocusAction::PressVk("vk2".to_string()),
            FocusAction::RawVkAction("raw2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_combined_virtual_key_and_raw_vk_action() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: Some(vec![("vk_notify".to_string(), "Tap".to_string())]),
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("browser".to_string()),
            FocusAction::PressVk("vk_browser".to_string()),
            FocusAction::RawVkAction("vk_notify".to_string(), "Tap".to_string()),
        ]
    );
}

#[test]
fn test_wildcard_pattern() {
    let rules = vec![rule(Some("*"), None, Some("any"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("anything", ""), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["any".to_string()]);
}

#[test]
fn test_regex_pattern() {
    let rules = vec![rule(Some("^(firefox|chrome)$"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    assert_eq!(
        get_layers(&handler.handle(&win("firefox", ""), "default").unwrap()),
        vec!["browser".to_string()]
    );
    assert_eq!(handler.handle(&win("chrome", ""), "default"), None);
    assert_eq!(
        get_layers(&handler.handle(&win("chromium", ""), "default").unwrap()),
        vec!["default".to_string()]
    );
}

#[test]
fn test_three_rules_fallthrough_all_layers_execute() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("app"), None, Some("layer2"))),
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec![
            "layer1".to_string(),
            "layer2".to_string(),
            "layer3".to_string(),
        ]
    );
}

#[test]
fn test_multiple_raw_vk_actions_per_rule_all_execute() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(
            Some("app"),
            vec![("a1", "Press"), ("a2", "Release")],
        )),
        rule_raw_vk(Some("app"), vec![("b1", "Tap"), ("b2", "Toggle")]),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("a1".to_string(), "Press".to_string()),
            ("a2".to_string(), "Release".to_string()),
            ("b1".to_string(), "Tap".to_string()),
            ("b2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_non_fallthrough_stops_chain() {
    // First rule matches but has fallthrough=false, should stop chain
    let rules = vec![
        rule(Some("app"), None, Some("layer1")), // fallthrough=false
        rule(Some("app"), None, Some("layer2")), // would match but shouldn't be reached
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // Only layer1 should be collected
    assert_eq!(get_layers(&actions), vec!["layer1".to_string()]);
}

#[test]
fn test_fallthrough_stops_at_non_fallthrough() {
    // First two rules have fallthrough, third doesn't - chain stops at third
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("app"), None, Some("layer2"))),
        rule(Some("app"), None, Some("layer3")), // fallthrough=false, stops here
        rule(Some("app"), None, Some("layer4")), // should not be reached
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec![
            "layer1".to_string(),
            "layer2".to_string(),
            "layer3".to_string(),
        ]
    );
    // layer4 should NOT be present
}

#[test]
fn test_non_matching_rules_skipped_in_fallthrough() {
    // Rules that don't match should be skipped even with fallthrough
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("other"), None, Some("layer2"))), // doesn't match
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer2 should be skipped because "other" doesn't match "app"
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer3".to_string(),]
    );
}

#[test]
fn test_non_matching_non_fallthrough_rule_does_not_stop_chain() {
    // A non-matching rule with fallthrough=false should NOT stop the chain
    // (only matching rules can stop the chain)
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule(Some("other"), None, Some("layer2")), // doesn't match, fallthrough=false
        rule(Some("app"), None, Some("layer3")),   // should still be reached
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer1 and layer3 should be collected; layer2 skipped (doesn't match)
    // The non-matching rule's fallthrough=false should NOT stop the chain
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer3".to_string(),]
    );
}
