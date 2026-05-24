use super::*;

mod fixtures;
pub(crate) use fixtures::*;

mod restart_or_shutdown;
mod logind_decode;
mod display_apply;
mod runtime_target;
mod persistent_dbus;
mod sni_runtime;
mod transition;
mod provider;
mod supervisor;
