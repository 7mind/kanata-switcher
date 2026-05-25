#[cfg(target_os = "linux")]
pub(crate) mod client;
#[cfg(target_os = "linux")]
pub(crate) mod server;
#[cfg(target_os = "linux")]
pub(crate) mod persistent;
#[cfg(target_os = "linux")]
pub(crate) use persistent::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlCommand {
    Restart,
    Pause,
    Unpause,
}

impl ControlCommand {
    pub(crate) fn dbus_method(self) -> &'static str {
        match self {
            ControlCommand::Restart => "Restart",
            ControlCommand::Pause => "Pause",
            ControlCommand::Unpause => "Unpause",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            ControlCommand::Restart => "restart",
            ControlCommand::Pause => "pause",
            ControlCommand::Unpause => "unpause",
        }
    }
}

/// Per-instance control dispatch mode. `Unicast` targets a single daemon bus
/// name; `Broadcast` enumerates all daemons in the `instances.*` namespace.
pub(crate) enum ControlDispatch {
    Unicast { bus_name: String },
    Broadcast,
}
