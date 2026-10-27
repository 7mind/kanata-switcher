use zbus::Connection;
use crate::broadcasters::{RestartHandle, ShutdownHandle};

#[derive(Clone)]
pub(crate) struct SniDbusControl {
    pub(crate) runtime_handle: tokio::runtime::Handle,
    pub(crate) connection: Connection,
    pub(crate) restart_handle: RestartHandle,
    pub(crate) shutdown_handle: ShutdownHandle,
    /// Per-instance daemon bus name to target with DBus control calls.
    pub(crate) daemon_bus_name: String,
}
