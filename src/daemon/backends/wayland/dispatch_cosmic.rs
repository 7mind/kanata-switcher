use super::{ToplevelWindow, WaylandState};
use super::protocols::cosmic_toplevel::{
    zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1},
    zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1},
};
use super::protocols::cosmic_workspace::{
    zcosmic_workspace_group_handle_v1::ZcosmicWorkspaceGroupHandleV1,
    zcosmic_workspace_handle_v1::ZcosmicWorkspaceHandleV1,
    zcosmic_workspace_manager_v1::ZcosmicWorkspaceManagerV1,
};
use wayland_client::{Connection as WaylandConnection, Dispatch, Proxy, QueueHandle};

impl Dispatch<ZcosmicToplevelInfoV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let zcosmic_toplevel_info_v1::Event::Toplevel { toplevel } = event {
            state
                .windows
                .insert(toplevel.id(), ToplevelWindow::default());
        }
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicToplevelInfoV1, [
        zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (ZcosmicToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicToplevelHandleV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        handle: &ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zcosmic_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.app_id = app_id;
                }
            }
            zcosmic_toplevel_handle_v1::Event::Title { title } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.title = title;
                }
            }
            zcosmic_toplevel_handle_v1::Event::State {
                state: handle_state,
            } => {
                // COSMIC: activated = 2
                let (chunks, _) = handle_state.as_chunks::<4>();
                let activated = chunks
                    .iter()
                    .map(|&chunk| u32::from_ne_bytes(chunk))
                    .any(|s| s == zcosmic_toplevel_handle_v1::State::Activated as u32);
                if activated {
                    state.active_window = Some(handle.id());
                } else if state.active_window.as_ref() == Some(&handle.id()) {
                    // Window lost activation - clear active_window
                    state.active_window = None;
                }
            }
            zcosmic_toplevel_handle_v1::Event::Closed => {
                state.windows.remove(&handle.id());
                if state.active_window.as_ref() == Some(&handle.id()) {
                    state.active_window = None;
                }
            }
            _ => {}
        }
    }
}

// Dispatch for workspace types (we ignore these events but need to handle them)
impl Dispatch<ZcosmicWorkspaceManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceManagerV1,
        _: super::protocols::cosmic_workspace::zcosmic_workspace_manager_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicWorkspaceManagerV1, [
        super::protocols::cosmic_workspace::zcosmic_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ZcosmicWorkspaceGroupHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicWorkspaceGroupHandleV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceGroupHandleV1,
        _: super::protocols::cosmic_workspace::zcosmic_workspace_group_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicWorkspaceGroupHandleV1, [
        super::protocols::cosmic_workspace::zcosmic_workspace_group_handle_v1::EVT_WORKSPACE_OPCODE => (ZcosmicWorkspaceHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicWorkspaceHandleV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceHandleV1,
        _: super::protocols::cosmic_workspace::zcosmic_workspace_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}
