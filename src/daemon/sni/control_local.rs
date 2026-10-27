use std::sync::{Arc, Mutex};
use crate::kanata::KanataClient;
use crate::focus::FocusHandler;
use crate::broadcasters::{StatusBroadcaster, PauseBroadcaster};
use crate::pause::UnpauseContext;
use crate::broadcasters::RestartHandle;
use crate::broadcasters::ShutdownHandle;

#[derive(Clone)]
pub(crate) struct SniLocalControl {
    pub(crate) runtime_handle: tokio::runtime::Handle,
    pub(crate) kanata: KanataClient,
    pub(crate) handler: Arc<Mutex<FocusHandler>>,
    pub(crate) status_broadcaster: StatusBroadcaster,
    pub(crate) pause_broadcaster: PauseBroadcaster,
    pub(crate) restart_handle: RestartHandle,
    pub(crate) shutdown_handle: ShutdownHandle,
    pub(crate) unpause_context: UnpauseContext,
}
