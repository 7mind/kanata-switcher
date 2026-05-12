use std::sync::{Arc, Mutex};
use crate::{
    focus::{FocusHandler, FocusActions, FocusAction},
    kanata::KanataClient,
    broadcasters::{PauseBroadcaster, StatusBroadcaster},
    config::WindowInfo,
    args::TrayFocusOnly,
};
use crate::{SniSettingsStore, SNI_DEFAULT_SHOW_FOCUS_ONLY};

pub(crate) fn resolve_sni_focus_only(
    override_value: Option<TrayFocusOnly>,
    settings: &mut SniSettingsStore,
) -> bool {
    if let Some(value) = override_value {
        return value.as_bool();
    }
    settings
        .read_focus_only()
        .unwrap_or(SNI_DEFAULT_SHOW_FOCUS_ONLY)
}

/// Execute focus actions in order
pub(crate) async fn execute_focus_actions(kanata: &KanataClient, actions: FocusActions) {
    for action in actions.actions {
        match action {
            FocusAction::ReleaseVk(vk) => {
                kanata.act_on_fake_key(&vk, "Release").await;
            }
            FocusAction::ChangeLayer(layer) => {
                kanata.change_layer(&layer).await;
            }
            FocusAction::PressVk(vk) => {
                kanata.act_on_fake_key(&vk, "Press").await;
            }
            FocusAction::RawVkAction(name, action) => {
                kanata.act_on_fake_key(&name, &action).await;
            }
        }
    }
}

pub(crate) fn extract_focus_layer(actions: &FocusActions) -> Option<String> {
    actions.actions.iter().fold(None, |last, action| {
        if let FocusAction::ChangeLayer(layer) = action {
            Some(layer.clone())
        } else {
            last
        }
    })
}

pub(crate) async fn update_status_for_focus(
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    win: &WindowInfo,
    kanata: &KanataClient,
    default_layer: &str,
) -> Option<FocusActions> {
    let (actions, virtual_keys, focus_layer) = {
        let mut handler = handler.lock().unwrap();
        let actions = handler.handle(win, default_layer);
        let virtual_keys = handler.current_virtual_keys();
        let focus_layer = actions
            .as_ref()
            .and_then(|focus_actions| extract_focus_layer(focus_actions));
        (actions, virtual_keys, focus_layer)
    };

    // Filter out invalid VKs before updating indicator
    let known_vks = kanata.known_virtual_keys().await;
    let valid_virtual_keys = KanataClient::filter_valid_virtual_keys(&known_vks, virtual_keys);
    status_broadcaster.update_virtual_keys(valid_virtual_keys);
    if let Some(layer) = focus_layer {
        if let Some(resolved_layer) = kanata.resolve_layer_name(&layer, false).await {
            status_broadcaster.update_focus_layer(resolved_layer);
        }
    }

    actions
}

pub(crate) async fn handle_focus_event(
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    pause_broadcaster: &PauseBroadcaster,
    win: &WindowInfo,
    kanata: &KanataClient,
    default_layer: &str,
) -> Option<FocusActions> {
    if pause_broadcaster.is_paused() {
        return None;
    }
    update_status_for_focus(handler, status_broadcaster, win, kanata, default_layer).await
}

pub(crate) fn native_terminal_window() -> WindowInfo {
    WindowInfo {
        class: String::new(),
        title: String::new(),
        is_native_terminal: true,
    }
}
