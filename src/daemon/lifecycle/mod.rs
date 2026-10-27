pub(crate) mod logind;
pub(crate) mod startup;
pub(crate) mod snapshot;


use crate::environ::{Environment, LifecycleSnapshot};
use crate::errors::DynError;
use logind::LogindLifecycleProvider;
use startup::StartupSnapshotProvider;

#[derive(Debug)]
pub(crate) enum LifecycleProvider {
    Logind(LogindLifecycleProvider),
    Startup(StartupSnapshotProvider),
}

impl LifecycleProvider {
    pub(crate) async fn build(env: Environment) -> Self {
        Self::build_with_logind_factory(env, LogindLifecycleProvider::new).await
    }

    pub(crate) async fn build_with_logind_factory<F, Fut>(env: Environment, logind_factory: F) -> Self
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<LogindLifecycleProvider, DynError>>,
    {
        match logind_factory().await {
            Ok(provider) => {
                println!("[Lifecycle] Provider=logind (continuous)");
                Self::Logind(provider)
            }
            Err(error) => {
                eprintln!(
                    "[Lifecycle] Provider=startup-snapshot (logind unavailable): {}",
                    error
                );
                Self::Startup(StartupSnapshotProvider::new(env))
            }
        }
    }

    pub(crate) fn is_continuous(&self) -> bool {
        matches!(self, Self::Logind(_))
    }

    pub(crate) async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        match self {
            Self::Logind(provider) => provider.next_snapshot().await,
            Self::Startup(provider) => provider.next_snapshot().await,
        }
    }
}
