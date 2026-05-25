#[cfg(target_os = "linux")]
pub(crate) mod logind;
pub(crate) mod startup;
#[cfg(target_os = "linux")]
pub(crate) mod snapshot;


use crate::environ::{Environment, LifecycleSnapshot};
use crate::errors::DynError;
use startup::StartupSnapshotProvider;

#[derive(Debug)]
pub(crate) enum LifecycleProvider {
    #[cfg(target_os = "linux")]
    Logind(LogindLifecycleProvider),
    Startup(StartupSnapshotProvider),
}

impl LifecycleProvider {
    pub(crate) async fn build(env: Environment) -> Self {
        #[cfg(target_os = "linux")]
        if let Ok(provider) = LogindLifecycleProvider::new().await {
            println!("[Lifecycle] Provider=logind (continuous)");
            return Self::Logind(provider);
        }

        Self::Startup(StartupSnapshotProvider::new(env))
    }

    pub(crate) fn is_continuous(&self) -> bool {
        #[cfg(target_os = "linux")]
        if matches!(self, Self::Logind(_)) {
            return true;
        }
        false
    }

    pub(crate) async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Logind(provider) => provider.next_snapshot().await,
            Self::Startup(provider) => provider.next_snapshot().await,
        }
    }
}
