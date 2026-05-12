use std::future::Future;
use std::os::unix::io::AsRawFd;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::io::unix::AsyncFd;
use x11rb::connection::Connection as X11Connection;
use x11rb::protocol::Event as X11Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as X11ConnectionExt, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::broadcasters::{PauseBroadcaster, StatusBroadcaster};
use crate::config::WindowInfo;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};
use crate::kanata::KanataClient;
use crate::ShutdownHandle;
use crate::backends::{BackendExit, BackendRunContext, FocusBackend, RawFdWatcher};

x11rb::atom_manager! {
    pub X11Atoms: X11AtomsCookie {
        _NET_WM_NAME,
        _NET_ACTIVE_WINDOW,
        UTF8_STRING,
    }
}

pub(crate) struct X11State {
    pub(crate) connection: RustConnection,
    pub(crate) root: Window,
    pub(crate) atoms: X11Atoms,
}

impl X11State {
    pub(crate) fn new(
        display_override: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (connection, screen_num) = x11rb::connect(display_override)?;
        let root = connection.setup().roots[screen_num].root;
        let atoms = X11Atoms::new(&connection)?.reply()?;

        // Subscribe to PropertyNotify events on root window
        let attrs = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        connection.change_window_attributes(root, &attrs)?;
        connection.flush()?;

        Ok(Self {
            connection,
            root,
            atoms,
        })
    }

    pub(crate) fn get_active_window_id(&self) -> Option<Window> {
        let prop_reply = self
            .connection
            .get_property(
                false,
                self.root,
                self.atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ == x11rb::NONE || prop_reply.value.len() != 4 {
            return None;
        }

        let arr: [u8; 4] = prop_reply.value.clone().try_into().ok()?;
        let winid = u32::from_le_bytes(arr);

        if winid == 0 { None } else { Some(winid) }
    }

    pub(crate) fn get_window_class(&self, window: Window) -> Option<String> {
        let reply = self
            .connection
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;

        if reply.value.is_empty() {
            return None;
        }

        // WM_CLASS format: "instance\0class\0"
        // We want just the class part (second element)
        let parts: Vec<&[u8]> = reply.value.split(|&b| b == 0).collect();
        if parts.len() >= 2 {
            String::from_utf8(parts[1].to_vec()).ok()
        } else if !parts.is_empty() {
            String::from_utf8(parts[0].to_vec()).ok()
        } else {
            None
        }
    }

    pub(crate) fn get_window_title(&self, window: Window) -> Option<String> {
        // Try _NET_WM_NAME first (UTF-8)
        let prop_reply = self
            .connection
            .get_property(
                false,
                window,
                self.atoms._NET_WM_NAME,
                self.atoms.UTF8_STRING,
                0,
                u32::MAX,
            )
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ != x11rb::NONE {
            return String::from_utf8(prop_reply.value).ok();
        }

        // Fallback to WM_NAME (Latin-1)
        let prop_reply = self
            .connection
            .get_property(
                false,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                0,
                u32::MAX,
            )
            .ok()?
            .reply()
            .ok()?;

        String::from_utf8(prop_reply.value).ok()
    }

    pub(crate) fn get_active_window(&self) -> WindowInfo {
        let Some(window_id) = self.get_active_window_id() else {
            return WindowInfo::default();
        };

        let class = self.get_window_class(window_id).unwrap_or_default();
        let title = self.get_window_title(window_id).unwrap_or_default();

        WindowInfo {
            class,
            title,
            is_native_terminal: false,
        }
    }
}

pub(crate) fn query_x11_active_window(
    x11_display_override: Option<&str>,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new(x11_display_override)?;
    Ok(state.get_active_window())
}

pub(crate) async fn run_x11(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    x11_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new(x11_display_override.as_deref())?;

    println!("[X11] Connected to display");

    let initial = state.get_active_window();
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

    println!("[X11] Listening for focus events...");

    let raw_fd = state.connection.stream().as_raw_fd();
    let async_fd = AsyncFd::new(RawFdWatcher::new(raw_fd))?;
    let mut shutdown_receiver = shutdown_handle.subscribe();

    // Event loop - wait for PropertyNotify events on _NET_ACTIVE_WINDOW
    loop {
        if *shutdown_receiver.borrow() {
            return Ok(());
        }

        while let Some(event) = state.connection.poll_for_event()? {
            match event {
                X11Event::PropertyNotify(e) if e.atom == state.atoms._NET_ACTIVE_WINDOW => {
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
                _ => {}
            }
        }

        let mut readiness = tokio::select! {
            _ = shutdown_receiver.changed() => {
                return Ok(());
            }
            readiness = async_fd.readable() => readiness?,
        };
        readiness.clear_ready();
    }
}

pub(crate) struct X11Backend;

impl FocusBackend for X11Backend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            run_x11(
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
