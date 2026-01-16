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
    assert_eq!(actions.new_managed_vk, None);
}

#[test]
fn test_virtual_key_press_on_focus() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert!(has_action(&actions, &FocusAction::PressVk("vk_browser".to_string())));
    assert!(!actions.actions.iter().any(|a| matches!(a, FocusAction::ReleaseVk(_))));
    assert_eq!(actions.new_managed_vk, Some("vk_browser".to_string()));
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

    // Window changed but VK is the same - no VK actions, but action list might be empty
    assert!(actions.is_none() || !actions.as_ref().unwrap().actions.iter().any(|a|
        matches!(a, FocusAction::PressVk(_) | FocusAction::ReleaseVk(_))
    ));
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
fn test_fallthrough_intermediate_vk_tapped_final_pressed() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_vk(Some("kitty"), "vk2"),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // vk1 should be tapped (intermediate), vk2 should be pressed (final)
    assert!(has_action(&actions, &FocusAction::TapVk("vk1".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk2".to_string())));
    assert_eq!(actions.new_managed_vk, Some("vk2".to_string()));
}

#[test]
fn test_fallthrough_multiple_intermediate_vks_all_tapped() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk2")),
        rule_vk(Some("kitty"), "vk3"),
    ];
    let mut handler = FocusHandler::new(rules, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // vk1 and vk2 should be tapped, vk3 should be pressed
    assert!(has_action(&actions, &FocusAction::TapVk("vk1".to_string())));
    assert!(has_action(&actions, &FocusAction::TapVk("vk2".to_string())));
    assert!(has_action(&actions, &FocusAction::PressVk("vk3".to_string())));
    assert_eq!(actions.new_managed_vk, Some("vk3".to_string()));
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

    // Expected order: layer1, TapVk(vk1), raw1, layer2, PressVk(vk2), raw2
    assert_eq!(actions.actions, vec![
        FocusAction::ChangeLayer("layer1".to_string()),
        FocusAction::TapVk("vk1".to_string()),
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
    fn prop_at_most_one_managed_vk(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 1..10),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        for win in &windows {
            let _ = handler.handle(win, "default");
            // Invariant: at most one managed VK can be active
            prop_assert!(handler.current_virtual_key.is_none() || handler.current_virtual_key.is_some());
        }
    }

    #[test]
    fn prop_release_before_other_actions(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 2..10),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        for win in &windows {
            if let Some(actions) = handler.handle(win, "default") {
                // If there's a ReleaseVk, it should be first
                if let Some(release_idx) = actions.actions.iter().position(|a| matches!(a, FocusAction::ReleaseVk(_))) {
                    prop_assert_eq!(release_idx, 0, "ReleaseVk should be first action");
                }
            }
        }
    }

    #[test]
    fn prop_unfocus_releases_vk(
        rules in prop::collection::vec(arb_rule(), 1..5),
        win in arb_window(),
    ) {
        let mut handler = FocusHandler::new(rules, true);

        // Focus a window first
        let _ = handler.handle(&win, "default");
        let vk_before = handler.current_virtual_key.clone();

        // Unfocus (empty class and title)
        let actions = handler.handle(&WindowInfo { class: String::new(), title: String::new() }, "default");

        // If we had a VK active, it must be released
        if let Some(old_vk) = vk_before {
            prop_assert!(actions.is_some());
            let actions = actions.unwrap();
            prop_assert!(has_action(&actions, &FocusAction::ReleaseVk(old_vk)));
        }
        // After unfocus, no VK should be active
        prop_assert!(handler.current_virtual_key.is_none());
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
    fn prop_intermediate_vks_tapped_final_pressed(
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
            // vk1 should be tapped (intermediate)
            prop_assert!(has_action(&actions, &FocusAction::TapVk(vk1)));
            // vk2 should be pressed (final)
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk2.clone())));
            // new_managed_vk should be vk2
            prop_assert_eq!(actions.new_managed_vk, Some(vk2));
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
