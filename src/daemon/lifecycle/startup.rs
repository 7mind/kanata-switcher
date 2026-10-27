use crate::environ::{Environment, LifecycleSnapshot, startup_environment_to_snapshot};

#[derive(Debug)]
pub(crate) struct StartupSnapshotProvider {
    snapshot: Option<LifecycleSnapshot>,
}

impl StartupSnapshotProvider {
    pub(crate) fn new(env: Environment) -> Self {
        Self {
            snapshot: Some(startup_environment_to_snapshot(env)),
        }
    }

    pub(crate) async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        self.snapshot.take()
    }
}
