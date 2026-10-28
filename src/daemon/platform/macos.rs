use std::sync::{Arc, Mutex};

use crate::backends::{BackendRunContext, FocusBackend};
use crate::broadcasters::{PauseBroadcaster, ShutdownHandle, StatusBroadcaster};
use crate::environ::RunOutcome;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::kanata::KanataClient;

pub(crate) async fn run(
    kanata: KanataClient,
    focus_handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    // Register before spawning so an early SIGTERM cannot bypass cleanup.
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let shutdown_handle_for_signal = shutdown_handle.clone();
    let signal_task = tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
        shutdown_handle_for_signal.request();
    });

    let backend = Box::new(crate::backends::macos::MacOsBackend);
    let ctx = BackendRunContext {
        kanata,
        focus_handler,
        status_broadcaster,
        pause_broadcaster,
        restart_handle: crate::broadcasters::RestartHandle::new(),
        shutdown_handle,
        effective_bus_name: String::new(),
        display_override: None,
    };
    let result = backend.run(ctx).await;
    signal_task.abort();
    result?;
    Ok(RunOutcome::Exit)
}

/// AppKit/NSWorkspace delivers workspace notifications through the process main
/// run loop. Tokio's async entry point alone never pumps that loop.
pub(super) fn run_main(future: impl std::future::Future<Output = ()> + Send + 'static) {
    use core_foundation_sys::runloop::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSWorkspace;
    use std::time::Duration;

    // Initialize workspace services on the actual OS main thread.
    autoreleasepool(|_| unsafe {
        let _ = NSWorkspace::sharedWorkspace();
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    let task = runtime.spawn(future);
    while !task.is_finished() {
        autoreleasepool(|_| unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0);
        });
        std::thread::park_timeout(Duration::from_millis(10));
    }
    runtime.block_on(task).expect("daemon task panicked");
}
