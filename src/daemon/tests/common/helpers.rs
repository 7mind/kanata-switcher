use super::*;

pub(crate) const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
pub(crate) static SNI_WATCHER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) async fn with_test_timeout<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("test timeout")
}

pub(crate) fn win(class: &str, title: &str) -> WindowInfo {
    WindowInfo {
        class: class.to_string(),
        title: title.to_string(),
        is_native_terminal: false,
    }
}

pub(crate) fn rule(class: Option<&str>, title: Option<&str>, layer: Option<&str>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: title.map(String::from),
        on_native_terminal: None,
        layer: layer.map(String::from),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }
}

pub(crate) fn rule_vk(class: Option<&str>, virtual_key: &str) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        on_native_terminal: None,
        layer: None,
        virtual_key: Some(virtual_key.to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }
}

pub(crate) fn rule_raw_vk(class: Option<&str>, raw_vk_action: Vec<(&str, &str)>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        on_native_terminal: None,
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

pub(crate) fn rule_with_fallthrough(mut r: Rule) -> Rule {
    r.fallthrough = true;
    r
}

/// Helper to check if actions contain a specific action
pub(crate) fn has_action(actions: &FocusActions, action: &FocusAction) -> bool {
    actions.actions.contains(action)
}

/// Helper to get all actions of a specific type
pub(crate) fn get_layers(actions: &FocusActions) -> Vec<String> {
    actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::ChangeLayer(l) = a {
                Some(l.clone())
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn get_raw_vk_actions(actions: &FocusActions) -> Vec<(String, String)> {
    actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::RawVkAction(n, act) = a {
                Some((n.clone(), act.clone()))
            } else {
                None
            }
        })
        .collect()
}
