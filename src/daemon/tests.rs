use super::*;
use proptest::prelude::*;

fn win(class: &str, title: &str) -> WindowInfo {
    WindowInfo {
        class: class.to_string(),
        title: title.to_string(),
    }
}

fn rule(class: Option<&str>, title: Option<&str>, layer: Option<&str>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: title.map(String::from),
        layer: layer.map(String::from),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }
}

fn rule_vk(class: Option<&str>, virtual_key: &str) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        layer: None,
        virtual_key: Some(virtual_key.to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }
}

fn rule_raw_vk(class: Option<&str>, raw_vk_action: Vec<(&str, &str)>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        layer: None,
        virtual_key: None,
        raw_vk_action: Some(
            raw_vk_action
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
        fallthrough: false,
    }
}

fn rule_with_fallthrough(mut r: Rule) -> Rule {
    r.fallthrough = true;
    r
}

/// Helper to check if actions contain a specific action
fn has_action(actions: &FocusActions, action: &FocusAction) -> bool {
    actions.actions.contains(action)
}

/// Helper to get all actions of a specific type
fn get_layers(actions: &FocusActions) -> Vec<String> {
    actions.actions.iter().filter_map(|a| {
        if let FocusAction::ChangeLayer(l) = a { Some(l.clone()) } else { None }
    }).collect()
}

fn get_raw_vk_actions(actions: &FocusActions) -> Vec<(String, String)> {
    actions.actions.iter().filter_map(|a| {
        if let FocusAction::RawVkAction(n, act) = a { Some((n.clone(), act.clone())) } else { None }
    }).collect()
}

// === Flow Tests ===

#[test]
fn test_basic_layer_match() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(actions.actions, vec![FocusAction::ChangeLayer("browser".to_string())]);
}

#[test]
fn test_no_match_uses_default() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(actions.actions, vec![FocusAction::ChangeLayer("default".to_string())]);
}

#[test]
fn test_same_window_no_action() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab1"), "default");
    assert_eq!(actions, None);
}

#[test]
fn test_title_change_triggers_action() {
    let rules = vec![
        rule(Some("kitty"), Some("vim"), Some("vim")),
        rule(Some("kitty"), None, Some("terminal")),
    ];
    let mut handler = FocusHandler::new(rules, true);

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
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("", ""), "default").unwrap();

    assert_eq!(actions.actions, vec![
        FocusAction::ReleaseVk("vk_browser".to_string()),
        FocusAction::ChangeLayer("default".to_string()),
    ]);
    assert_eq!(actions.new_managed_vks, Vec::<String>::new());
}

#[test]
fn test_virtual_key_press_on_focus() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert!(has_action(&actions, &FocusAction::PressVk("vk_browser".to_string())));
    assert!(!actions.actions.iter().any(|a| matches!(a, FocusAction::ReleaseVk(_))));
    assert_eq!(actions.new_managed_vks, vec!["vk_browser".to_string()]);
}

#[test]
fn test_virtual_key_release_on_switch() {
    let rules = vec![
        rule_vk(Some("firefox"), "vk_browser"),
        rule_vk(Some("kitty"), "vk_terminal"),
    ];
    let mut handler = FocusHandler::new(rules, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Release comes before Press in the action list
    let release_idx = actions.actions.iter().position(|a| matches!(a, FocusAction::ReleaseVk(_)));
    let press_idx = actions.actions.iter().position(|a| matches!(a, FocusAction::PressVk(_)));
    assert!(release_idx.unwrap() < press_idx.unwrap());

    assert!(has_action(&actions, &FocusAction::ReleaseVk("vk_browser".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk_terminal".to_string())));
}

#[test]
fn test_virtual_key_no_change_no_press() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab2"), "default");

    // Window changed but VK is the same - no VK actions (VK already held)
    assert!(actions.is_none() || !actions.as_ref().unwrap().actions.iter().any(|a|
        matches!(a, FocusAction::PressVk(_) | FocusAction::ReleaseVk(_))
    ));
}

#[test]
fn test_partial_vk_set_change_only_releases_removed() {
    // Two rules with fallthrough: vk1 and vk2 are both held
    // Then switch to a window that only matches vk2 - only vk1 should be released
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: Some("both".to_string()),
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, true);

    // Focus window that matches both rules - both VKs pressed
    let actions = handler.handle(&win("app", "both"), "default").unwrap();
    assert!(has_action(&actions, &FocusAction::PressVk("vk1".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk2".to_string())));
    assert_eq!(actions.new_managed_vks, vec!["vk1".to_string(), "vk2".to_string()]);

    // Focus window that only matches second rule - only vk1 should be released, vk2 stays held
    let actions = handler.handle(&win("app", "other"), "default").unwrap();
    assert!(has_action(&actions, &FocusAction::ReleaseVk("vk1".to_string())));
    assert!(!has_action(&actions, &FocusAction::ReleaseVk("vk2".to_string())));
    assert!(!has_action(&actions, &FocusAction::PressVk("vk2".to_string()))); // vk2 already held
    assert_eq!(actions.new_managed_vks, vec!["vk2".to_string()]);
}

#[test]
fn test_unfocus_releases_multiple_vks_in_reverse_order() {
    // Multiple VKs held should be released in reverse order (bottom-to-top)
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: None,
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            layer: None,
            virtual_key: Some("vk3".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, true);

    // Focus window - all three VKs pressed in order
    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(actions.new_managed_vks, vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]);

    // Unfocus - all VKs should be released in reverse order (vk3, vk2, vk1)
    let actions = handler.handle(&win("", ""), "default").unwrap();
    let release_actions: Vec<_> = actions.actions.iter()
        .filter_map(|a| if let FocusAction::ReleaseVk(vk) = a { Some(vk.clone()) } else { None })
        .collect();
    assert_eq!(release_actions, vec!["vk3".to_string(), "vk2".to_string(), "vk1".to_string()]);
}

#[test]
fn test_raw_vk_action_fires_on_focus() {
    let rules = vec![rule_raw_vk(Some("firefox"), vec![("vk1", "Tap"), ("vk2", "Toggle")])];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(get_raw_vk_actions(&actions), vec![
        ("vk1".to_string(), "Tap".to_string()),
        ("vk2".to_string(), "Toggle".to_string()),
    ]);
}

#[test]
fn test_fallthrough_collects_all_layers() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("kitty"), None, Some("layer1"))),
        rule(Some("kitty"), None, Some("layer2")),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both layers should be in the action list, in order
    assert_eq!(get_layers(&actions), vec!["layer1".to_string(), "layer2".to_string()]);
}

#[test]
fn test_fallthrough_collects_all_raw_vk_actions() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(Some("kitty"), vec![("vk1", "Press")])),
        rule_raw_vk(Some("kitty"), vec![("vk2", "Tap")]),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(get_raw_vk_actions(&actions), vec![
        ("vk1".to_string(), "Press".to_string()),
        ("vk2".to_string(), "Tap".to_string()),
    ]);
}

#[test]
fn test_fallthrough_all_vks_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_vk(Some("kitty"), "vk2"),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both vk1 and vk2 should be pressed (all matched VKs are held)
    assert!(has_action(&actions, &FocusAction::PressVk("vk1".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk2".to_string())));
    assert_eq!(actions.new_managed_vks, vec!["vk1".to_string(), "vk2".to_string()]);
}

#[test]
fn test_fallthrough_multiple_vks_all_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk2")),
        rule_vk(Some("kitty"), "vk3"),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // All three VKs should be pressed and held
    assert!(has_action(&actions, &FocusAction::PressVk("vk1".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk2".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk3".to_string())));
    assert_eq!(actions.new_managed_vks, vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]);
}

#[test]
fn test_fallthrough_action_order_preserved() {
    // Test that actions from each rule are in order: layer, vk, raw_vk
    let rules = vec![
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            layer: Some("layer1".to_string()),
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: Some(vec![("raw1".to_string(), "Tap".to_string())]),
            fallthrough: true,
        },
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            layer: Some("layer2".to_string()),
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: Some(vec![("raw2".to_string(), "Toggle".to_string())]),
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Expected order: layer1, PressVk(vk1), raw1, layer2, PressVk(vk2), raw2
    // All matched VKs are pressed (not tapped)
    assert_eq!(actions.actions, vec![
        FocusAction::ChangeLayer("layer1".to_string()),
        FocusAction::PressVk("vk1".to_string()),
        FocusAction::RawVkAction("raw1".to_string(), "Tap".to_string()),
        FocusAction::ChangeLayer("layer2".to_string()),
        FocusAction::PressVk("vk2".to_string()),
        FocusAction::RawVkAction("raw2".to_string(), "Toggle".to_string()),
    ]);
}

#[test]
fn test_combined_virtual_key_and_raw_vk_action() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: Some(vec![("vk_notify".to_string(), "Tap".to_string())]),
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(actions.actions, vec![
        FocusAction::ChangeLayer("browser".to_string()),
        FocusAction::PressVk("vk_browser".to_string()),
        FocusAction::RawVkAction("vk_notify".to_string(), "Tap".to_string()),
    ]);
}

#[test]
fn test_wildcard_pattern() {
    let rules = vec![rule(Some("*"), None, Some("any"))];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("anything", ""), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["any".to_string()]);
}

#[test]
fn test_regex_pattern() {
    let rules = vec![rule(Some("^(firefox|chrome)$"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, true);

    assert_eq!(get_layers(&handler.handle(&win("firefox", ""), "default").unwrap()), vec!["browser".to_string()]);
    assert_eq!(get_layers(&handler.handle(&win("chrome", ""), "default").unwrap()), vec!["browser".to_string()]);
    assert_eq!(get_layers(&handler.handle(&win("chromium", ""), "default").unwrap()), vec!["default".to_string()]);
}

#[test]
fn test_three_rules_fallthrough_all_layers_execute() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("app"), None, Some("layer2"))),
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(get_layers(&actions), vec![
        "layer1".to_string(),
        "layer2".to_string(),
        "layer3".to_string(),
    ]);
}

#[test]
fn test_multiple_raw_vk_actions_per_rule_all_execute() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(Some("app"), vec![("a1", "Press"), ("a2", "Release")])),
        rule_raw_vk(Some("app"), vec![("b1", "Tap"), ("b2", "Toggle")]),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(get_raw_vk_actions(&actions), vec![
        ("a1".to_string(), "Press".to_string()),
        ("a2".to_string(), "Release".to_string()),
        ("b1".to_string(), "Tap".to_string()),
        ("b2".to_string(), "Toggle".to_string()),
    ]);
}

#[test]
fn test_non_fallthrough_stops_chain() {
    // First rule matches but has fallthrough=false, should stop chain
    let rules = vec![
        rule(Some("app"), None, Some("layer1")),  // fallthrough=false
        rule(Some("app"), None, Some("layer2")),  // would match but shouldn't be reached
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, true);

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
        rule(Some("app"), None, Some("layer3")),  // fallthrough=false, stops here
        rule(Some("app"), None, Some("layer4")),  // should not be reached
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(get_layers(&actions), vec![
        "layer1".to_string(),
        "layer2".to_string(),
        "layer3".to_string(),
    ]);
    // layer4 should NOT be present
}

#[test]
fn test_non_matching_rules_skipped_in_fallthrough() {
    // Rules that don't match should be skipped even with fallthrough
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("other"), None, Some("layer2"))),  // doesn't match
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer2 should be skipped because "other" doesn't match "app"
    assert_eq!(get_layers(&actions), vec![
        "layer1".to_string(),
        "layer3".to_string(),
    ]);
}

#[test]
fn test_non_matching_non_fallthrough_rule_does_not_stop_chain() {
    // A non-matching rule with fallthrough=false should NOT stop the chain
    // (only matching rules can stop the chain)
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule(Some("other"), None, Some("layer2")),  // doesn't match, fallthrough=false
        rule(Some("app"), None, Some("layer3")),    // should still be reached
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer1 and layer3 should be collected; layer2 skipped (doesn't match)
    // The non-matching rule's fallthrough=false should NOT stop the chain
    assert_eq!(get_layers(&actions), vec![
        "layer1".to_string(),
        "layer3".to_string(),
    ]);
}

// === Property Tests ===

fn arb_class() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("firefox".to_string()),
        Just("kitty".to_string()),
        Just("code".to_string()),
        Just("alacritty".to_string()),
        Just("".to_string()),
        "[a-z]{1,10}".prop_map(String::from),
    ]
}

fn arb_title() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        Just("vim".to_string()),
        Just("bash".to_string()),
        "[a-zA-Z0-9 ]{0,20}".prop_map(String::from),
    ]
}

fn arb_layer() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("default".to_string()),
        Just("browser".to_string()),
        Just("terminal".to_string()),
        Just("vim".to_string()),
        "[a-z]{1,8}".prop_map(String::from),
    ]
}

fn arb_vk_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("vk1".to_string()),
        Just("vk2".to_string()),
        Just("vk_browser".to_string()),
        Just("vk_terminal".to_string()),
    ]
}

fn arb_vk_action() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("Press".to_string()),
        Just("Release".to_string()),
        Just("Tap".to_string()),
        Just("Toggle".to_string()),
    ]
}

fn arb_rule() -> impl Strategy<Value = Rule> {
    (
        prop::option::of(arb_class()),
        prop::option::of(arb_title()),
        prop::option::of(arb_layer()),
        prop::option::of(arb_vk_name()),
        prop::option::of(prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..3)),
        any::<bool>(),
    )
        .prop_map(|(class, title, layer, vk, raw_vk, fallthrough)| Rule {
            class,
            title,
            layer,
            virtual_key: vk,
            raw_vk_action: raw_vk,
            fallthrough,
        })
}

fn arb_window() -> impl Strategy<Value = WindowInfo> {
    (arb_class(), arb_title()).prop_map(|(class, title)| WindowInfo { class, title })
}

proptest! {
    #[test]
    fn prop_managed_vks_consistent(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 1..10),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        for win in &windows {
            let _ = handler.handle(win, "default");
            // Just verify the handler state is consistent (Vec is always valid)
            prop_assert!(handler.current_virtual_keys.len() <= 10); // sanity bound
        }
    }

    #[test]
    fn prop_releases_before_presses(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 2..10),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        for win in &windows {
            if let Some(actions) = handler.handle(win, "default") {
                // All ReleaseVk actions should come before any PressVk actions
                let first_press_idx = actions.actions.iter().position(|a| matches!(a, FocusAction::PressVk(_)));
                let last_release_idx = actions.actions.iter().rposition(|a| matches!(a, FocusAction::ReleaseVk(_)));

                if let (Some(press_idx), Some(release_idx)) = (first_press_idx, last_release_idx) {
                    prop_assert!(release_idx < press_idx, "All releases should come before presses");
                }
            }
        }
    }

    #[test]
    fn prop_unfocus_releases_all_vks(
        rules in prop::collection::vec(arb_rule(), 1..5),
        win in arb_window(),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        // Focus a window first
        let _ = handler.handle(&win, "default");
        let vks_before = handler.current_virtual_keys.clone();

        // Unfocus (empty class and title)
        let actions = handler.handle(&WindowInfo { class: String::new(), title: String::new() }, "default");

        // All previously active VKs must be released
        if !vks_before.is_empty() {
            prop_assert!(actions.is_some());
            let actions = actions.unwrap();
            for old_vk in &vks_before {
                prop_assert!(has_action(&actions, &FocusAction::ReleaseVk(old_vk.clone())));
            }
        }
        // After unfocus, no VKs should be active
        prop_assert!(handler.current_virtual_keys.is_empty());
    }

    #[test]
    fn prop_fallthrough_collects_all_raw_vk(
        base_class in arb_class(),
        raw_vk1 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
        raw_vk2 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk1.is_empty() { None } else { Some(raw_vk1.clone()) },
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk2.is_empty() { None } else { Some(raw_vk2.clone()) },
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, true);
        let win = WindowInfo { class: base_class, title: String::new() };

        if let Some(actions) = handler.handle(&win, "default") {
            let expected: Vec<_> = raw_vk1.into_iter().chain(raw_vk2).collect();
            prop_assert_eq!(get_raw_vk_actions(&actions), expected);
        }
    }

    #[test]
    fn prop_fallthrough_collects_all_layers(
        base_class in arb_class(),
        layer1 in arb_layer(),
        layer2 in arb_layer(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: Some(layer1.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: Some(layer2.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, true);
        let win = WindowInfo { class: base_class, title: String::new() };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both layers should be collected
            prop_assert_eq!(get_layers(&actions), vec![layer1, layer2]);
        }
    }

    #[test]
    fn prop_all_matched_vks_pressed_and_held(
        base_class in arb_class(),
        vk1 in arb_vk_name(),
        vk2 in arb_vk_name(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: None,
                virtual_key: Some(vk1.clone()),
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                layer: None,
                virtual_key: Some(vk2.clone()),
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, true);
        let win = WindowInfo { class: base_class, title: String::new() };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both vk1 and vk2 should be pressed (all matched VKs are held)
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk1.clone())));
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk2.clone())));
            // new_managed_vks should contain both
            prop_assert_eq!(actions.new_managed_vks, vec![vk1, vk2]);
        }
    }
}

// === GNOME Extension State Parsing Tests ===

#[test]
fn test_gnome_extension_state_enabled_f64() {
    // GNOME Shell returns state as f64, 1.0 = ENABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert("state".to_string(), OwnedValue::try_from(Value::F64(1.0)).unwrap());

    let status = parse_gnome_extension_state(&body);
    assert!(status.active, "state=1.0 should be active");
    assert!(status.enabled, "state=1.0 should be enabled");
    assert!(status.installed, "D-Bus response means installed");
}

#[test]
fn test_gnome_extension_state_disabled_f64() {
    // State 2.0 = DISABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert("state".to_string(), OwnedValue::try_from(Value::F64(2.0)).unwrap());

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "state=2.0 should not be active");
    assert!(!status.enabled, "state=2.0 should not be enabled");
}

#[test]
fn test_gnome_extension_state_missing() {
    // No state field - should default to not enabled
    let body = HashMap::new();

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "missing state should not be active");
    assert!(!status.enabled, "missing state should not be enabled");
}
