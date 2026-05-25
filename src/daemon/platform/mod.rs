use std::sync::{Arc, Mutex};

use crate::args::Args;
use crate::broadcasters::{PauseBroadcaster, ShutdownHandle, StatusBroadcaster};
use crate::config::Config;
use crate::environ::{Environment, RunOutcome};
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::kanata::{KanataClient, ShutdownGuard};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub(crate) async fn run(
    args: Args,
    config: Config,
    quiet_focus: bool,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let shutdown_handle = ShutdownHandle::new();
    let kanata = KanataClient::new(
        &args.host,
        args.port,
        config.default_layer,
        args.quiet,
        status_broadcaster.clone(),
    );
    kanata.connect_with_retry().await;

    let focus_handler = Arc::new(Mutex::new(FocusHandler::new(
        config.rules.clone(),
        config.native_terminal_rule.clone(),
        quiet_focus,
    )));

    let _shutdown_guard = ShutdownGuard::new(kanata.clone());

    #[cfg(target_os = "linux")]
    return linux::run(args, kanata, focus_handler, status_broadcaster, pause_broadcaster, shutdown_handle).await;
    #[cfg(target_os = "macos")]
    return macos::run(kanata, focus_handler, status_broadcaster, pause_broadcaster, shutdown_handle).await;
    #[cfg(target_os = "windows")]
    return windows::run(kanata, focus_handler, status_broadcaster, pause_broadcaster, shutdown_handle).await;

    #[allow(unreachable_code)]
    Ok(RunOutcome::Exit)
}
