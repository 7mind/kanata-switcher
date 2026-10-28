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
