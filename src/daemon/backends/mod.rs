pub(crate) mod gnome;
pub(crate) mod kde;
pub(crate) mod wayland;
pub(crate) mod x11;
pub(crate) mod linux_console;

pub(crate) use wayland::*;
pub(crate) use x11::*;

use std::future::Future;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use zbus::Connection;
use crate::broadcasters::{PauseBroadcaster, RestartHandle, ShutdownHandle, StatusBroadcaster};
use crate::config::WindowInfo;
use crate::display_override::resolve_display_override_for_environment;
use crate::environ::{Environment, RunOutcome};
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event, native_terminal_window};
use crate::kanata::KanataClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendExit {
    Restart,
    Exit,
}

pub(crate) fn map_run_outcome_to_backend_exit(outcome: RunOutcome) -> BackendExit {
    match outcome {
        RunOutcome::Restart => BackendExit::Restart,
        RunOutcome::Exit => BackendExit::Exit,
    }
}

pub(crate) struct BackendRunContext {
    pub(crate) kanata: KanataClient,
    pub(crate) focus_handler: Arc<Mutex<FocusHandler>>,
    pub(crate) status_broadcaster: StatusBroadcaster,
    pub(crate) pause_broadcaster: PauseBroadcaster,
    pub(crate) restart_handle: RestartHandle,
    pub(crate) shutdown_handle: ShutdownHandle,
    pub(crate) effective_bus_name: String,
    pub(crate) display_override: Option<String>,
}

pub(crate) trait FocusBackend: Send + 'static {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>>;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RawFdWatcher {
    fd: RawFd,
}

impl RawFdWatcher {
    pub(crate) fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl AsRawFd for RawFdWatcher {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

pub(crate) async fn query_focus_for_env(
    env: Environment,
    connection: Option<&Connection>,
    is_kde6: bool,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    match env {
        Environment::Gnome => {
            let conn = connection.expect("GNOME focus query requires session connection");
            gnome::query_gnome_focus(conn).await
        }
        Environment::Kde => {
            let conn = connection.expect("KDE focus query requires session connection");
            kde::probe::query_kde_focus(conn, is_kde6).await
        }
        Environment::Wayland => {
            let display_override = resolve_display_override_for_environment(env, "Focus").await;
            tokio::task::block_in_place(move || {
                query_wayland_active_window(display_override.as_deref())
            })
        }
        Environment::X11 => {
            let display_override = resolve_display_override_for_environment(env, "Focus").await;
            tokio::task::block_in_place(move || {
                query_x11_active_window(display_override.as_deref())
            })
        }
        Environment::LinuxConsoleWithLogind => Ok(native_terminal_window()),
        Environment::Unknown => Ok(WindowInfo::default()),
    }
}

pub(crate) async fn apply_focus_for_env(
    env: Environment,
    connection: Option<&Connection>,
    is_kde6: bool,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    pause_broadcaster: &PauseBroadcaster,
    kanata: &KanataClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let win = query_focus_for_env(env, connection, is_kde6).await?;
    let default_layer = kanata.default_layer().await.unwrap_or_default();
    if let Some(actions) = handle_focus_event(
        handler,
        status_broadcaster,
        pause_broadcaster,
        &win,
        kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(kanata, actions).await;
    }
    Ok(())
}
