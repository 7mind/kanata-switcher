use super::*;
use proptest::prelude::*;

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

fn arb_nonempty_class() -> impl Strategy<Value = String> {
    arb_class().prop_filter("non-empty class", |class| !class.is_empty())
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
        prop::option::of(prop::collection::vec(
            (arb_vk_name(), arb_vk_action()),
            0..3,
        )),
        any::<bool>(),
    )
        .prop_map(|(class, title, layer, vk, raw_vk, fallthrough)| Rule {
            class,
            title,
            on_native_terminal: None,
            layer,
            virtual_key: vk,
            raw_vk_action: raw_vk,
            fallthrough,
        })
}

fn arb_window() -> impl Strategy<Value = WindowInfo> {
    (arb_class(), arb_title()).prop_map(|(class, title)| WindowInfo {
        class,
        title,
        is_native_terminal: false,
    })
}

proptest! {
    #[test]
    fn prop_managed_vks_consistent(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 1..10),
    ) {
        let mut handler = FocusHandler::new(rules, None, true);

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
        let mut handler = FocusHandler::new(rules, None, true);

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
        let mut handler = FocusHandler::new(rules, None, true);

        // Focus a window first
        let _ = handler.handle(&win, "default");
        let vks_before = handler.current_virtual_keys.clone();

        // Unfocus (empty class and title)
        let actions = handler.handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: false,
            },
            "default",
        );

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
        base_class in arb_nonempty_class(),
        raw_vk1 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
        raw_vk2 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk1.is_empty() { None } else { Some(raw_vk1.clone()) },
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk2.is_empty() { None } else { Some(raw_vk2.clone()) },
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            let expected: Vec<_> = raw_vk1.into_iter().chain(raw_vk2).collect();
            prop_assert_eq!(get_raw_vk_actions(&actions), expected);
        }
    }

    #[test]
    fn prop_fallthrough_collects_all_layers(
        base_class in arb_nonempty_class(),
        layer1 in arb_layer(),
        layer2 in arb_layer(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: Some(layer1.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: Some(layer2.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both layers should be collected
            prop_assert_eq!(get_layers(&actions), vec![layer1, layer2]);
        }
    }

    #[test]
    fn prop_all_matched_vks_pressed_and_held(
        base_class in arb_nonempty_class(),
        vk1 in arb_vk_name(),
        vk2 in arb_vk_name(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: Some(vk1.clone()),
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: Some(vk2.clone()),
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both vk1 and vk2 should be pressed (all matched VKs are held)
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk1.clone())));
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk2.clone())));
            // new_managed_vks should contain both
            prop_assert_eq!(actions.new_managed_vks, vec![vk1, vk2]);
        }
    }
}
