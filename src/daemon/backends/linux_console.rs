use std::future::Future;
use std::pin::Pin;
use crate::backends::{apply_focus_for_env, BackendExit, BackendRunContext, FocusBackend, map_run_outcome_to_backend_exit};
use crate::broadcasters::wait_for_restart_or_shutdown;
use crate::environ::Environment;
use crate::errors::DynError;

pub(crate) struct LinuxConsoleBackend;

impl FocusBackend for LinuxConsoleBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            apply_focus_for_env(
                Environment::LinuxConsoleWithLogind,
                None,
                false,
                &ctx.focus_handler,
                &ctx.status_broadcaster,
                &ctx.pause_broadcaster,
                &ctx.kanata,
            )
            .await?;
            let outcome =
                wait_for_restart_or_shutdown(&ctx.restart_handle, &ctx.shutdown_handle).await;
            Ok(map_run_outcome_to_backend_exit(outcome))
        })
    }
}
