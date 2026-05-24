use std::future::Future;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use block2::StackBlock;
use core_foundation_sys::runloop::{
    CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun, CFRunLoopStop,
};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::NSNotification;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::backends::{BackendExit, BackendRunContext, FocusBackend};
use crate::config::WindowInfo;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};
use crate::kanata::KanataClient;
use crate::broadcasters::{PauseBroadcaster, ShutdownHandle, StatusBroadcaster};

fn current_bundle_id() -> String {
    unsafe {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .and_then(|app| app.bundleIdentifier())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

pub(crate) fn current_window_info() -> WindowInfo {
    WindowInfo {
        class: current_bundle_id(),
        title: String::new(),
        is_native_terminal: false,
    }
}

fn window_info_from_notification(notification: NonNull<NSNotification>) -> WindowInfo {
    unsafe {
        let bundle_id = notification
            .as_ref()
            .userInfo()
            .and_then(|info| info.objectForKey(NSWorkspaceApplicationKey))
            .and_then(|value| Retained::cast::<NSRunningApplication>(value).bundleIdentifier())
            .map(|id| id.to_string())
            .unwrap_or_else(current_bundle_id);

        WindowInfo {
            class: bundle_id,
            title: String::new(),
            is_native_terminal: false,
        }
    }
}

fn spawn_frontmost_app_stream() -> (UnboundedReceiver<WindowInfo>, Option<usize>) {
    let (event_tx, event_rx) = unbounded_channel();
    let (run_loop_tx, run_loop_rx) = mpsc::channel::<usize>();

    thread::spawn(move || unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        let run_loop = CFRunLoopGetCurrent();
        let _ = run_loop_tx.send(run_loop as usize);

        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &StackBlock::new(move |notification: NonNull<NSNotification>| {
                let _ = event_tx.send(window_info_from_notification(notification));
            }),
        );

        CFRunLoopRun();
        center.removeObserver(&observer);
    });

    (
        event_rx,
        run_loop_rx.recv().ok(),
    )
}

async fn run_macos(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<(), DynError> {
    println!("[macOS] Starting focus watcher (NSWorkspace notifications)");

    let initial = current_window_info();
    println!("[macOS] Initial frontmost app: \"{}\"", initial.class);
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &initial,
        &kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(&kanata, actions).await;
    }

    let (mut event_rx, run_loop_ptr) = spawn_frontmost_app_stream();
    let mut shutdown_rx = shutdown_handle.subscribe();

    println!("[macOS] Listening for frontmost-app activation events...");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                println!("[macOS] Shutting down focus watcher");
                if let Some(ptr) = run_loop_ptr {
                    unsafe { CFRunLoopStop(ptr as CFRunLoopRef); }
                }
                return Ok(());
            }
            maybe_window = event_rx.recv() => {
                let Some(win) = maybe_window else {
                    return Ok(());
                };

                let default_layer = kanata.default_layer_sync();
                if let Some(actions) = handle_focus_event(
                    &handler,
                    &status_broadcaster,
                    &pause_broadcaster,
                    &win,
                    &kanata,
                    &default_layer,
                )
                .await
                {
                    execute_focus_actions(&kanata, actions).await;
                }
            }
        }
    }
}

pub(crate) struct MacOsBackend;

impl FocusBackend for MacOsBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            run_macos(
                ctx.kanata,
                ctx.focus_handler,
                ctx.status_broadcaster,
                ctx.pause_broadcaster,
                ctx.shutdown_handle,
            )
            .await?;
            Ok(BackendExit::Exit)
        })
    }
}
