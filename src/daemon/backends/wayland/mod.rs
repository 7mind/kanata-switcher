pub(crate) mod protocols;
pub(crate) mod dispatch_common;
pub(crate) mod dispatch_wlr;
pub(crate) mod dispatch_cosmic;


use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::os::fd::AsFd;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
use tokio::io::unix::AsyncFd;
use wayland_client::{
    Connection as WaylandConnection,
    backend::{ObjectId, WaylandError},
    globals::registry_queue_init,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
};
use crate::broadcasters::{PauseBroadcaster, StatusBroadcaster};
use crate::backends::{BackendExit, BackendRunContext, FocusBackend, RawFdWatcher};
use crate::config::WindowInfo;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};
use crate::kanata::KanataClient;
use crate::ShutdownHandle;
use protocols::cosmic_toplevel::zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1;

#[cfg(test)]
static WAYLAND_QUERY_COUNTER: AtomicUsize = AtomicUsize::new(0);

// === Wayland Toplevel State ===

#[derive(Default)]
pub(crate) struct ToplevelWindow {
    pub(crate) app_id: String,
    pub(crate) title: String,
}

#[derive(Default)]
pub(crate) struct WaylandState {
    pub(crate) windows: HashMap<ObjectId, ToplevelWindow>,
    pub(crate) active_window: Option<ObjectId>,
}

impl WaylandState {
    pub(crate) fn get_active_window(&self) -> WindowInfo {
        self.active_window
            .as_ref()
            .and_then(|id| self.windows.get(id))
            .map(|w| WindowInfo {
                class: w.app_id.clone(),
                title: w.title.clone(),
                is_native_terminal: false,
            })
            .unwrap_or_default()
    }
}

pub(crate) fn resolve_wayland_socket_path(
    wayland_display: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let socket_name = PathBuf::from(wayland_display);
    if socket_name.is_absolute() {
        return Ok(socket_name);
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("XDG_RUNTIME_DIR is not set")?;
    if !runtime_dir.is_absolute() {
        return Err("XDG_RUNTIME_DIR must be an absolute path".into());
    }

    Ok(runtime_dir.join(socket_name))
}

pub(crate) fn connect_wayland_with_display_override(
    wayland_display_override: Option<&str>,
) -> Result<WaylandConnection, Box<dyn std::error::Error + Send + Sync>> {
    match wayland_display_override {
        Some(display) => {
            let socket_path = resolve_wayland_socket_path(display)?;
            let socket = UnixStream::connect(socket_path)?;
            Ok(WaylandConnection::from_socket(socket)?)
        }
        None => Ok(WaylandConnection::connect_to_env()?),
    }
}

pub(crate) fn query_wayland_active_window(
    wayland_display_override: Option<&str>,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(test)]
    {
        WAYLAND_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    }
    let connection = connect_wayland_with_display_override(wayland_display_override)?;
    let (globals, mut queue) = registry_queue_init::<WaylandState>(&connection)?;
    let mut state = WaylandState::default();

    if globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&queue.handle(), 1..=3, ())
        .is_err()
        && globals
            .bind::<ZcosmicToplevelInfoV1, _, _>(&queue.handle(), 1..=1, ())
            .is_err()
    {
        return Err(
            "No supported toplevel protocol (wlr-foreign-toplevel or cosmic-toplevel-info)".into(),
        );
    }

    for _ in 0..5 {
        queue.roundtrip(&mut state)?;
        if state.active_window.is_some() {
            break;
        }
    }
    Ok(state.get_active_window())
}

#[cfg(test)]
pub(crate) fn wayland_query_count() -> usize {
    WAYLAND_QUERY_COUNTER.load(Ordering::SeqCst)
}

// === Wayland Backend ===

#[derive(Debug, Clone, Copy)]
enum WaylandProtocol {
    Wlr,
    Cosmic,
}

pub(crate) async fn run_wayland(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    wayland_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = connect_wayland_with_display_override(wayland_display_override.as_deref())?;
    let (globals, mut queue) = registry_queue_init::<WaylandState>(&connection)?;

    let mut state = WaylandState::default();

    // Try wlr protocol first, fall back to cosmic
    let protocol = if globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&queue.handle(), 1..=3, ())
        .is_ok()
    {
        WaylandProtocol::Wlr
    } else if globals
        .bind::<ZcosmicToplevelInfoV1, _, _>(&queue.handle(), 1..=1, ())
        .is_ok()
    {
        WaylandProtocol::Cosmic
    } else {
        return Err(
            "No supported toplevel protocol (wlr-foreign-toplevel or cosmic-toplevel-info)".into(),
        );
    };

    println!("[Wayland] Using {:?} toplevel protocol", protocol);

    // Initial roundtrip to populate state
    queue.roundtrip(&mut state)?;

    println!("[Wayland] Listening for focus events...");

    let raw_fd = connection.as_fd().as_raw_fd();
    let async_fd = AsyncFd::new(RawFdWatcher::new(raw_fd))?;
    let mut shutdown_receiver = shutdown_handle.subscribe();

    let win = state.get_active_window();
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

    loop {
        if *shutdown_receiver.borrow() {
            return Ok(());
        }

        let dispatched = queue.dispatch_pending(&mut state)?;
        if dispatched > 0 {
            let win = state.get_active_window();
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
            continue;
        }

        connection.flush()?;
        let guard = match queue.prepare_read() {
            Some(guard) => guard,
            None => continue,
        };

        let mut readiness = tokio::select! {
            _ = shutdown_receiver.changed() => {
                return Ok(());
            }
            readiness = async_fd.readable() => readiness?,
        };

        let read_result = guard.read();
        readiness.clear_ready();

        match read_result {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                eprintln!("[Wayland] Read error: {}", error);
                return Err(error.into());
            }
        }

        let _ = queue.dispatch_pending(&mut state)?;
        let win = state.get_active_window();
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

pub(crate) struct WaylandBackend;

impl FocusBackend for WaylandBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            run_wayland(
                ctx.kanata,
                ctx.focus_handler,
                ctx.status_broadcaster,
                ctx.pause_broadcaster,
                ctx.display_override,
                ctx.shutdown_handle,
            )
            .await?;
            Ok(BackendExit::Exit)
        })
    }
}
