//! Shared native backend loop. Event sources send invalidations, not stale snapshots.
use tokio::sync::mpsc::Receiver;

use crate::backends::{BackendExit, BackendRunContext};
use crate::config::WindowInfo;
use crate::errors::DynError;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};

async fn sync_focus(ctx: &BackendRunContext, query: &impl Fn() -> WindowInfo) {
    let default_layer = ctx.kanata.default_layer().await.unwrap_or_default();
    if let Some(actions) = handle_focus_event(
        &ctx.focus_handler,
        &ctx.status_broadcaster,
        &ctx.pause_broadcaster,
        &query(),
        &ctx.kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(&ctx.kanata, actions).await;
    }
}

pub(super) async fn run(
    ctx: BackendRunContext,
    mut events: Receiver<()>,
    query: impl Fn() -> WindowInfo,
) -> Result<BackendExit, DynError> {
    // The caller registers the native listener before entering this function.
    // A fresh query also makes queued startup events and coalescing harmless.
    let mut shutdown = ctx.shutdown_handle.subscribe();
    let mut restart = ctx.restart_handle.subscribe();
    let mut pause = ctx.pause_broadcaster.subscribe();
    let result = async {
        if *shutdown.borrow() { return Ok(BackendExit::Exit); }
        if *restart.borrow() { return Ok(BackendExit::Restart); }
        sync_focus(&ctx, &query).await;
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => return Ok(BackendExit::Exit),
                _ = restart.changed() => return Ok(BackendExit::Restart),
                changed = pause.changed() => {
                    if changed.is_err() { return Err("Native pause channel closed".into()); }
                    let is_paused = *pause.borrow_and_update();
                    if is_paused {
                        release_keys(&ctx).await;
                        let default = ctx.kanata.default_layer().await.unwrap_or_default();
                        ctx.kanata.change_layer(&default).await;
                    } else {
                        release_keys(&ctx).await;
                        sync_focus(&ctx, &query).await;
                    }
                }
                event = events.recv() => {
                    if event.is_none() { return Err("Native focus event source stopped unexpectedly".into()); }
                    sync_focus(&ctx, &query).await;
                }
            }
        }
    }.await;

    // Graceful stop/restart must not leave auto-managed virtual keys held.
    release_keys(&ctx).await;
    result
}

async fn release_keys(ctx: &BackendRunContext) {
    let keys = {
        let mut handler = ctx.focus_handler.lock().unwrap();
        let keys = handler.current_virtual_keys();
        handler.reset();
        keys
    };
    for key in keys.iter().rev() {
        ctx.kanata.act_on_fake_key(key, "Release").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        broadcasters::{PauseBroadcaster, RestartHandle, ShutdownHandle, StatusBroadcaster},
        config::Rule,
        focus::FocusHandler,
        integration_tests::common::{KanataMessage, MockKanataServer, wait_for_kanata_message},
        kanata::KanataClient,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn startup_unpause_and_shutdown_use_current_focus_and_release_keys() {
        let server = MockKanataServer::start();
        let status = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".into()),
            true,
            status.clone(),
        );
        kanata.connect_with_retry().await;
        let handler = Arc::new(Mutex::new(FocusHandler::new(
            vec![Rule {
                class: Some("^native$".into()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".into()),
                virtual_key: Some("vk_browser".into()),
                raw_vk_action: None,
                fallthrough: false,
            }],
            None,
            true,
        )));
        let shutdown = ShutdownHandle::new();
        let pause = PauseBroadcaster::new();
        let context = BackendRunContext {
            kanata: kanata.clone(),
            focus_handler: handler,
            status_broadcaster: status,
            pause_broadcaster: pause.clone(),
            restart_handle: RestartHandle::new(),
            shutdown_handle: shutdown.clone(),
            effective_bus_name: String::new(),
            display_override: None,
        };
        let (events, receiver) = tokio::sync::mpsc::channel(1);
        let current = Arc::new(Mutex::new("native".to_string()));
        let query_current = current.clone();
        let task = tokio::spawn(run(context, receiver, move || WindowInfo {
            class: query_current.lock().unwrap().clone(),
            ..WindowInfo::default()
        }));
        let expect_layer = |name: &str| {
            wait_for_kanata_message(
                &server,
                KanataMessage::ChangeLayer { new: name.into() },
                Duration::from_secs(2),
            )
        };
        let expect_vk = |action: &str| {
            wait_for_kanata_message(
                &server,
                KanataMessage::ActOnFakeKey {
                    name: "vk_browser".into(),
                    action: action.into(),
                },
                Duration::from_secs(2),
            )
        };
        expect_layer("browser");
        expect_vk("Press");
        pause.set_paused(true);
        expect_vk("Release");
        expect_layer("default");
        *current.lock().unwrap() = "other".into();
        events.send(()).await.unwrap();
        assert!(
            server.recv_timeout(Duration::from_millis(100)).is_none(),
            "paused focus must not execute actions"
        );
        // Return to the same app while paused; no new event after unpause.
        *current.lock().unwrap() = "native".into();
        pause.set_paused(false);
        expect_layer("browser");
        expect_vk("Press");
        shutdown.request();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            BackendExit::Exit
        );
        expect_vk("Release");
        // Ensure the peer can stop without waiting for the runtime to drop the reader.
        kanata.pause_disconnect().await;
    }

    #[tokio::test]
    async fn already_requested_shutdown_exits_without_applying_initial_focus() {
        let shutdown = ShutdownHandle::new();
        shutdown.request();
        let status = StatusBroadcaster::new();
        let context = BackendRunContext {
            kanata: KanataClient::new("127.0.0.1", 0, Some("default".into()), true, status.clone()),
            focus_handler: Arc::new(Mutex::new(FocusHandler::new(vec![], None, true))),
            status_broadcaster: status,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle: shutdown,
            effective_bus_name: String::new(),
            display_override: None,
        };
        let (_events, receiver) = tokio::sync::mpsc::channel(1);
        assert_eq!(
            run(context, receiver, || panic!(
                "must not query focus after shutdown"
            ))
            .await
            .unwrap(),
            BackendExit::Exit
        );
    }
    #[tokio::test]
    async fn unexpected_source_exit_is_an_error() {
        let status = StatusBroadcaster::new();
        let context = BackendRunContext {
            kanata: KanataClient::new("127.0.0.1", 0, Some("default".into()), true, status.clone()),
            focus_handler: Arc::new(Mutex::new(FocusHandler::new(vec![], None, true))),
            status_broadcaster: status,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle: ShutdownHandle::new(),
            effective_bus_name: String::new(),
            display_override: None,
        };
        let (events, receiver) = tokio::sync::mpsc::channel(1);
        drop(events);
        let error = run(context, receiver, WindowInfo::default)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("event source stopped"));
    }
}
