use tokio::sync::watch;
use crate::broadcasters::{StatusSnapshot, LayerSource};

pub(crate) struct MenuRefresh {
    sender: watch::Sender<u64>,
    version: u64,
}

impl MenuRefresh {
    pub(crate) fn new() -> (Self, watch::Receiver<u64>) {
        let (sender, receiver) = watch::channel(0u64);
        (Self { sender, version: 0 }, receiver)
    }

    pub(crate) fn notify(&mut self) {
        self.version += 1;
        self.sender.send_replace(self.version);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SniIndicatorState {
    pub(crate) last_status: StatusSnapshot,
    pub(crate) focus_status: StatusSnapshot,
    pub(crate) paused: bool,
    pub(crate) show_focus_only: bool,
    pub(crate) menu_revision: u64,
}

impl SniIndicatorState {
    pub(crate) fn new(initial: StatusSnapshot, show_focus_only: bool) -> Self {
        Self {
            last_status: initial.clone(),
            focus_status: initial,
            paused: false,
            show_focus_only,
            menu_revision: 0,
        }
    }

    pub(crate) fn update_status(&mut self, snapshot: StatusSnapshot) {
        if snapshot.layer_source == LayerSource::Focus {
            self.focus_status = snapshot.clone();
        }
        self.last_status = snapshot;
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub(crate) fn toggle_focus_only(&mut self) {
        self.show_focus_only = !self.show_focus_only;
    }

    pub(crate) fn focus_only_enabled(&self) -> bool {
        self.show_focus_only
    }

    pub(crate) fn bump_menu_revision(&mut self) {
        self.menu_revision = self.menu_revision.wrapping_add(1);
    }

    pub(crate) fn display_status(&self) -> StatusSnapshot {
        if self.paused {
            return self.last_status.clone();
        }
        if self.show_focus_only {
            return self.focus_status.clone();
        }
        self.last_status.clone()
    }
}
