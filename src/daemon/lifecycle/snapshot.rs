use crate::environ::{LifecycleSnapshot, SessionKind};

pub(crate) fn snapshot_no_session() -> LifecycleSnapshot {
    LifecycleSnapshot {
        active: false,
        session_type: String::new(),
        session_kind: SessionKind::NoSession,
    }
}
