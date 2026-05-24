use std::sync::{Arc, Mutex};
use zbus::Connection;
use crate::{
    environ::Environment,
    kanata::KanataClient,
    focus::FocusHandler,
    broadcasters::{PauseBroadcaster, StatusBroadcaster},
};
use crate::backends::apply_focus_for_env;

#[derive(Clone)]
pub(crate) struct UnpauseContext {
    pub(crate) env: Environment,
    pub(crate) connection: Option<Connection>,
    pub(crate) is_kde6: bool,
}

pub(crate) fn local_sni_unpause_context(env: Environment) -> UnpauseContext {
    match env {
        Environment::Wayland | Environment::X11 => UnpauseContext {
            env,
            connection: None,
            is_kde6: false,
        },
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => {
            panic!(
                "[SNI] Local control created for unsupported environment: {:?}",
                env
            )
        }
    }
}

pub(crate) fn pause_daemon(
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    runtime_handle: &tokio::runtime::Handle,
    request_label: &str,
) {
    if !pause_broadcaster.set_paused(true) {
        println!("[Pause] Pause requested {} (already paused)", request_label);
        return;
    }
    println!("[Pause] Pausing daemon");
    let virtual_keys = {
        let mut handler = handler.lock().unwrap();
        let keys = handler.current_virtual_keys();
        handler.reset();
        keys
    };
    let status_broadcaster = status_broadcaster.clone();
    let kanata = kanata.clone();
    runtime_handle.block_on(async move {
        let default_layer = kanata.default_layer().await.unwrap_or_default();

        for vk in virtual_keys.iter().rev() {
            kanata.act_on_fake_key(vk, "Release").await;
        }

        if !default_layer.is_empty() {
            let _ = kanata.change_layer(&default_layer).await;
        }

        status_broadcaster.set_paused_status(default_layer);
        kanata.pause_disconnect().await;
    });
}

pub(crate) fn unpause_daemon(
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    runtime_handle: &tokio::runtime::Handle,
    request_label: &str,
) {
    record_unpause_request_environment_for_test(env);
    if !pause_broadcaster.set_paused(false) {
        println!(
            "[Pause] Unpause requested {} (already running)",
            request_label
        );
        return;
    }
    println!("[Pause] Resuming daemon");
    let pause_broadcaster = pause_broadcaster.clone();
    let handler = handler.clone();
    let status_broadcaster = status_broadcaster.clone();
    let kanata = kanata.clone();
    runtime_handle.block_on(async move {
        kanata.unpause_connect().await;
        if let Err(error) = apply_focus_for_env(
            env,
            connection.as_ref(),
            is_kde6,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        {
            panic!("[Pause] Failed to refresh focus after unpause: {}", error);
        }
    });
}

#[cfg(test)]
pub(crate) static TEST_LAST_UNPAUSE_REQUEST_ENV: std::sync::Mutex<Option<Environment>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn record_unpause_request_environment_for_test(env: Environment) {
    *TEST_LAST_UNPAUSE_REQUEST_ENV.lock().unwrap() = Some(env);
}

#[cfg(not(test))]
pub(crate) fn record_unpause_request_environment_for_test(_env: Environment) {}

#[cfg(test)]
pub(crate) fn take_unpause_request_environment_for_test() -> Option<Environment> {
    TEST_LAST_UNPAUSE_REQUEST_ENV.lock().unwrap().take()
}
