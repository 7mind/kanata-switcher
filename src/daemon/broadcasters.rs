use tokio::sync::watch;

use crate::environ::{Environment, RunOutcome};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusSnapshot {
    pub(crate) layer: String,
    pub(crate) virtual_keys: Vec<String>,
    pub(crate) layer_source: LayerSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayerSource {
    Focus,
    External,
}

impl LayerSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            LayerSource::Focus => "focus",
            LayerSource::External => "external",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StatusBroadcaster {
    pub(crate) sender: watch::Sender<StatusSnapshot>,
}

#[derive(Clone, Debug)]
pub(crate) struct RestartHandle {
    pub(crate) sender: watch::Sender<bool>,
}

#[derive(Clone, Debug)]
pub(crate) struct PauseBroadcaster {
    pub(crate) sender: watch::Sender<bool>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeEnvironmentBroadcaster {
    pub(crate) sender: watch::Sender<Environment>,
}

#[derive(Clone, Debug)]
pub(crate) struct ShutdownHandle {
    pub(crate) sender: watch::Sender<bool>,
}

impl RestartHandle {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    pub(crate) fn request(&self) {
        self.sender.send_replace(true);
    }
}

impl ShutdownHandle {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    pub(crate) fn request(&self) {
        self.sender.send_replace(true);
    }
}

impl PauseBroadcaster {
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    pub(crate) fn is_paused(&self) -> bool {
        *self.sender.borrow()
    }

    pub(crate) fn set_paused(&self, paused: bool) -> bool {
        let current = *self.sender.borrow();
        if current == paused {
            return false;
        }
        self.sender.send_replace(paused);
        true
    }
}

impl RuntimeEnvironmentBroadcaster {
    pub(crate) fn new(initial: Environment) -> Self {
        let (sender, _) = watch::channel(initial);
        Self { sender }
    }

    pub(crate) fn current(&self) -> Environment {
        *self.sender.borrow()
    }

    pub(crate) fn set_current(&self, env: Environment) {
        self.sender.send_replace(env);
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Environment> {
        self.sender.subscribe()
    }
}

impl StatusBroadcaster {
    pub(crate) fn new() -> Self {
        let initial = StatusSnapshot {
            layer: String::new(),
            virtual_keys: Vec::new(),
            layer_source: LayerSource::External,
        };
        let (sender, _) = watch::channel(initial);
        Self { sender }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<StatusSnapshot> {
        self.sender.subscribe()
    }

    pub(crate) fn snapshot(&self) -> StatusSnapshot {
        self.sender.borrow().clone()
    }

    pub(crate) fn update_layer(&self, layer: String, source: LayerSource) {
        self.update(|state| {
            state.layer = layer;
            state.layer_source = source;
        });
    }

    pub(crate) fn update_virtual_keys(&self, virtual_keys: Vec<String>) {
        self.update(|state| {
            state.virtual_keys = virtual_keys;
        });
    }

    pub(crate) fn update_focus_layer(&self, layer: String) {
        let mut next = self.sender.borrow().clone();
        next.layer = layer;
        next.layer_source = LayerSource::Focus;
        self.sender.send_replace(next);
    }

    pub(crate) fn set_paused_status(&self, layer: String) {
        let mut next = self.sender.borrow().clone();
        next.layer = layer;
        next.layer_source = LayerSource::External;
        next.virtual_keys = Vec::new();
        self.sender.send_replace(next);
    }

    pub(crate) fn update<F>(&self, updater: F)
    where
        F: FnOnce(&mut StatusSnapshot),
    {
        let current = self.sender.borrow().clone();
        let mut next = current.clone();
        updater(&mut next);
        if next != current {
            self.sender.send_replace(next);
        }
    }
}

pub(crate) async fn wait_for_restart_or_shutdown(
    restart_handle: &RestartHandle,
    shutdown_handle: &ShutdownHandle,
) -> RunOutcome {
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();

    if *shutdown_receiver.borrow() {
        return RunOutcome::Exit;
    }
    if *restart_receiver.borrow() {
        return RunOutcome::Restart;
    }

    tokio::select! {
        _ = shutdown_receiver.changed() => RunOutcome::Exit,
        _ = restart_receiver.changed() => {
            if *shutdown_receiver.borrow() {
                RunOutcome::Exit
            } else {
                RunOutcome::Restart
            }
        }
    }
}
