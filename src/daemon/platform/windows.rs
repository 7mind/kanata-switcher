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
    let shutdown_handle_for_signal = shutdown_handle.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
        eprintln!("[Signal] Received Ctrl+C");
        shutdown_handle_for_signal.request();
    });

    let backend = Box::new(crate::backends::windows::WindowsBackend);
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
    backend.run(ctx).await?;
    Ok(RunOutcome::Exit)
}
