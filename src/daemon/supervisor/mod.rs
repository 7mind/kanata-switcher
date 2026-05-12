pub(crate) mod capabilities;
pub(crate) use capabilities::*;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use crate::broadcasters::*;
use crate::display_override::*;
use crate::environ::*;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::kanata::KanataClient;
use crate::lifecycle::*;
use crate::backends::{BackendExit, BackendRunContext, FocusBackend};
use crate::backends::gnome::GnomeBackend;
use crate::backends::kde::KdeBackend;
use crate::backends::wayland::WaylandBackend;
use crate::backends::x11::X11Backend;
use crate::backends::linux_console::LinuxConsoleBackend;

pub(crate) const WAYLAND_CAPABILITY_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(crate) struct BackendContext {
    pub(crate) kanata: KanataClient,
    pub(crate) handler: Arc<Mutex<FocusHandler>>,
    pub(crate) status_broadcaster: StatusBroadcaster,
    pub(crate) restart_handle: RestartHandle,
    pub(crate) pause_broadcaster: PauseBroadcaster,
    pub(crate) runtime_environment: RuntimeEnvironmentBroadcaster,
    pub(crate) install_gnome_extension: bool,
    pub(crate) gnome_setup_completed: Arc<AtomicBool>,
    pub(crate) gnome_setup_hook: Arc<dyn Fn(bool) + Send + Sync>,
    /// Effective per-instance well-known DBus name owned by this daemon.
    /// Threaded into the GNOME signal-match filter and the KWin script template.
    pub(crate) effective_dbus_name: String,
}

pub(crate) struct BackendHandle {
    pub(crate) kind: BackendKind,
    pub(crate) shutdown_handle: ShutdownHandle,
    pub(crate) join_handle: Option<tokio::task::JoinHandle<Result<BackendExit, DynError>>>,
    pub(crate) finished_rx: watch::Receiver<bool>,
}

impl BackendHandle {
    pub(crate) fn is_finished(&self) -> bool {
        match &self.join_handle {
            Some(join_handle) => {
                let jh: &tokio::task::JoinHandle<Result<BackendExit, DynError>> = join_handle;
                jh.is_finished()
            }
            None => true,
        }
    }

    pub(crate) async fn take_join_result(&mut self) -> Result<BackendExit, DynError> {
        let join_handle: tokio::task::JoinHandle<Result<BackendExit, DynError>> = self
            .join_handle
            .take()
            .expect("backend join handle missing");
        let result = join_handle.await.map_err(|error| {
            let message = format!("[Lifecycle] Backend task join failure: {}", error);
            Box::<dyn std::error::Error + Send + Sync>::from(message)
        })?;
        result
    }

    pub(crate) async fn stop(&mut self) -> Result<BackendExit, DynError> {
        if self.join_handle.is_none() {
            return Ok(BackendExit::Exit);
        }
        self.shutdown_handle.request();
        self.take_join_result().await
    }

    pub(crate) fn finished_receiver(&self) -> watch::Receiver<bool> {
        self.finished_rx.clone()
    }
}

pub(crate) fn runtime_target_label(target: RuntimeTarget) -> &'static str {
    match target {
        RuntimeTarget::Idle => "idle",
        RuntimeTarget::Backend(BackendKind::Gnome) => "gnome",
        RuntimeTarget::Backend(BackendKind::Kde) => "kde",
        RuntimeTarget::Backend(BackendKind::Wayland) => "wayland",
        RuntimeTarget::Backend(BackendKind::X11) => "x11",
        RuntimeTarget::Backend(BackendKind::LinuxConsole) => "linux-console",
    }
}

pub(crate) fn runtime_target_is_wayland_family(target: RuntimeTarget) -> bool {
    matches!(
        target,
        RuntimeTarget::Backend(BackendKind::Gnome)
            | RuntimeTarget::Backend(BackendKind::Kde)
            | RuntimeTarget::Backend(BackendKind::Wayland)
    )
}

pub(crate) fn runtime_target_to_environment(target: RuntimeTarget) -> Environment {
    match target {
        RuntimeTarget::Backend(BackendKind::Gnome) => Environment::Gnome,
        RuntimeTarget::Backend(BackendKind::Kde) => Environment::Kde,
        RuntimeTarget::Backend(BackendKind::Wayland) => Environment::Wayland,
        RuntimeTarget::Backend(BackendKind::X11) => Environment::X11,
        RuntimeTarget::Backend(BackendKind::LinuxConsole) => Environment::LinuxConsoleWithLogind,
        RuntimeTarget::Idle => Environment::Unknown,
    }
}

pub(crate) async fn ensure_runtime_gnome_extension_setup(context: &BackendContext) -> Result<(), DynError> {
    if context.gnome_setup_completed.load(Ordering::SeqCst) {
        return Ok(());
    }

    let setup_hook = context.gnome_setup_hook.clone();
    let install_gnome_extension = context.install_gnome_extension;
    tokio::task::spawn_blocking(move || (setup_hook)(install_gnome_extension))
        .await
        .map_err(|error| -> DynError {
            format!("[GNOME] Extension setup task failed: {}", error).into()
        })?;

    context.gnome_setup_completed.store(true, Ordering::SeqCst);
    Ok(())
}

pub(crate) async fn start_backend(
    kind: BackendKind,
    context: &BackendContext,
) -> Result<BackendHandle, DynError> {
    let shutdown_handle = ShutdownHandle::new();
    let (finished_tx, finished_rx) = watch::channel(false);

    let display_override = match kind {
        BackendKind::Wayland | BackendKind::X11 => {
            resolve_display_override_for_backend_kind(kind, "Lifecycle").await
        }
        _ => None,
    };

    let backend: Box<dyn FocusBackend> = match kind {
        BackendKind::Gnome => Box::new(GnomeBackend),
        BackendKind::Kde => Box::new(KdeBackend),
        BackendKind::Wayland => Box::new(WaylandBackend),
        BackendKind::X11 => Box::new(X11Backend),
        BackendKind::LinuxConsole => Box::new(LinuxConsoleBackend),
    };

    let run_ctx = BackendRunContext {
        kanata: context.kanata.clone(),
        focus_handler: context.handler.clone(),
        status_broadcaster: context.status_broadcaster.clone(),
        pause_broadcaster: context.pause_broadcaster.clone(),
        restart_handle: context.restart_handle.clone(),
        shutdown_handle: shutdown_handle.clone(),
        effective_bus_name: context.effective_dbus_name.clone(),
        display_override,
    };

    let join_handle = {
        let task_finished = finished_tx.clone();
        tokio::spawn(async move {
            let result = backend.run(run_ctx).await;
            let _ = task_finished.send(true);
            result
        })
    };

    Ok(BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    })
}

pub(crate) struct SupervisorState {
    pub(crate) current_target: RuntimeTarget,
    pub(crate) backend: Option<BackendHandle>,
}

impl SupervisorState {
    pub(crate) fn new() -> Self {
        Self {
            current_target: RuntimeTarget::Idle,
            backend: None,
        }
    }
}

#[cfg(test)]
pub(crate) async fn transition_runtime_target(
    state: &mut SupervisorState,
    desired_target: RuntimeTarget,
    context: &BackendContext,
    reason: &str,
) -> Result<(), DynError> {
    transition_runtime_target_with_starter(
        state,
        desired_target,
        context,
        reason,
        |kind, context| async move { start_backend(kind, &context).await },
    )
    .await
}

pub(crate) async fn transition_runtime_target_with_starter<F, Fut>(
    state: &mut SupervisorState,
    desired_target: RuntimeTarget,
    context: &BackendContext,
    reason: &str,
    starter: F,
) -> Result<(), DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
{
    if state.current_target == desired_target {
        return Ok(());
    }
    let requires_session_bus = target_requires_session_bus(desired_target);

    println!(
        "[LifecycleTransition] from={} to={} session_bus_required={} reason={}",
        runtime_target_label(state.current_target),
        runtime_target_label(desired_target),
        requires_session_bus,
        reason
    );

    if let Some(mut backend) = state.backend.take() {
        let exit = backend.stop().await?;
        if exit == BackendExit::Restart {
            context.restart_handle.request();
        }
    }

    if desired_target == RuntimeTarget::Backend(BackendKind::Gnome) {
        ensure_runtime_gnome_extension_setup(context).await?;
    }

    if let RuntimeTarget::Backend(kind) = desired_target {
        let backend = starter(kind, context.clone()).await?;
        state.backend = Some(backend);
    }

    state.current_target = desired_target;
    context
        .runtime_environment
        .set_current(runtime_target_to_environment(desired_target));
    Ok(())
}

pub(crate) async fn stop_current_backend(
    state: &mut SupervisorState,
    context: &BackendContext,
) -> Result<(), DynError> {
    if let Some(mut backend) = state.backend.take() {
        let exit = backend.stop().await?;
        if exit == BackendExit::Restart {
            context.restart_handle.request();
        }
    }
    state.current_target = RuntimeTarget::Idle;
    context
        .runtime_environment
        .set_current(Environment::Unknown);
    Ok(())
}

pub(crate) async fn run_lifecycle_supervisor(
    provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, DynError> {
    run_lifecycle_supervisor_with_starter(
        provider,
        context,
        restart_handle,
        shutdown_handle,
        |kind, context| async move { start_backend(kind, &context).await },
    )
    .await
}

pub(crate) async fn run_lifecycle_supervisor_with_starter<F, Fut>(
    provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    starter: F,
) -> Result<RunOutcome, DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
{
    run_lifecycle_supervisor_with_starter_and_resolver(
        provider,
        context,
        restart_handle,
        shutdown_handle,
        starter,
        |snapshot| async move { resolve_runtime_target_for_snapshot(&snapshot).await },
        WAYLAND_CAPABILITY_RECHECK_INTERVAL,
    )
    .await
}

pub(crate) async fn run_lifecycle_supervisor_with_starter_and_resolver<F, Fut, R, RFut>(
    mut provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    starter: F,
    resolver: R,
    wayland_capability_recheck_interval: Duration,
) -> Result<RunOutcome, DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
    R: Fn(LifecycleSnapshot) -> RFut,
    RFut: std::future::Future<Output = Result<RuntimeTarget, DynError>>,
{
    let mut state = SupervisorState::new();
    let allow_wayland_capability_recheck = provider.is_continuous();
    context
        .runtime_environment
        .set_current(runtime_target_to_environment(state.current_target));
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();
    let mut provider_open = true;
    let mut last_snapshot: Option<LifecycleSnapshot> = None;

    loop {
        if *shutdown_receiver.borrow() {
            stop_current_backend(&mut state, &context).await?;
            return Ok(RunOutcome::Exit);
        }
        if *restart_receiver.borrow() {
            stop_current_backend(&mut state, &context).await?;
            return Ok(RunOutcome::Restart);
        }

        if let Some(outcome) = poll_finished_backend_outcome(&mut state).await? {
            return Ok(outcome);
        }

        let mut backend_finished = state
            .backend
            .as_ref()
            .map(|backend| backend.finished_receiver());
        if provider_open {
            tokio::select! {
                _ = shutdown_receiver.changed() => {}
                _ = restart_receiver.changed() => {}
                _ = wait_for_backend_completion_signal(&mut backend_finished) => {}
                _ = wait_for_wayland_capability_recheck(
                    allow_wayland_capability_recheck,
                    last_snapshot.as_ref(),
                    wayland_capability_recheck_interval,
                ) => {
                    let snapshot = last_snapshot
                        .clone()
                        .expect("capability recheck requires last snapshot");
                    match resolver(snapshot).await {
                        Ok(desired_target) => {
                            transition_runtime_target_with_starter(
                                &mut state,
                                desired_target,
                                &context,
                                "wayland-capability-recheck",
                                &starter,
                            )
                            .await?;
                        }
                        Err(error) => {
                            if runtime_target_is_wayland_family(state.current_target) {
                                eprintln!(
                                    "[Lifecycle] Keeping {} backend after capability recheck resolver error: {}",
                                    runtime_target_label(state.current_target),
                                    error
                                );
                            } else {
                                eprintln!(
                                    "[Lifecycle] Falling back to generic Wayland after capability recheck resolver error: {}",
                                    error
                                );
                                transition_runtime_target_with_starter(
                                    &mut state,
                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                    &context,
                                    "wayland-capability-recheck-fallback-after-resolver-error",
                                    &starter,
                                )
                                .await?;
                            }
                        }
                    }
                }
                next_snapshot = provider.next_snapshot() => {
                    match next_snapshot {
                        Some(snapshot) => {
                            last_snapshot = Some(snapshot.clone());
                            match resolver(snapshot.clone()).await {
                                Ok(desired_target) => {
                                    let reason = format!(
                                        "active={} type={} kind={:?}",
                                        snapshot.active,
                                        snapshot.session_type,
                                        snapshot.session_kind
                                    );
                                    transition_runtime_target_with_starter(
                                        &mut state,
                                        desired_target,
                                        &context,
                                        &reason,
                                        &starter,
                                    )
                                    .await?;
                                }
                                Err(error) => {
                                    if snapshot.session_kind == SessionKind::GraphicalWayland {
                                        if allow_wayland_capability_recheck {
                                            if runtime_target_is_wayland_family(state.current_target) {
                                                eprintln!(
                                                    "[Lifecycle] Keeping {} backend after continuous wayland resolver error: {}",
                                                    runtime_target_label(state.current_target),
                                                    error
                                                );
                                            } else {
                                                eprintln!(
                                                    "[Lifecycle] Falling back to generic Wayland after continuous resolver error: {}",
                                                    error
                                                );
                                                transition_runtime_target_with_starter(
                                                    &mut state,
                                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                                    &context,
                                                    "continuous-wayland-fallback-after-resolver-error",
                                                    &starter,
                                                )
                                                .await?;
                                            }
                                        } else {
                                            eprintln!(
                                                "[Lifecycle] Falling back to generic Wayland after startup resolver error: {}",
                                                error
                                            );
                                            transition_runtime_target_with_starter(
                                                &mut state,
                                                RuntimeTarget::Backend(BackendKind::Wayland),
                                                &context,
                                                "startup-wayland-fallback-after-resolver-error",
                                                &starter,
                                            )
                                            .await?;
                                        }
                                    } else if allow_wayland_capability_recheck {
                                        eprintln!(
                                            "[Lifecycle] Skipping transition after resolver error: {}",
                                            error
                                        );
                                    } else {
                                        return Err(format!(
                                            "[Lifecycle] Startup lifecycle target resolution failed: {}",
                                            error
                                        )
                                        .into());
                                    }
                                }
                            }
                        }
                        None => {
                            provider_open = false;
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                _ = shutdown_receiver.changed() => {}
                _ = restart_receiver.changed() => {}
                _ = wait_for_backend_completion_signal(&mut backend_finished) => {}
                _ = wait_for_wayland_capability_recheck(
                    allow_wayland_capability_recheck,
                    last_snapshot.as_ref(),
                    wayland_capability_recheck_interval,
                ) => {
                    let snapshot = last_snapshot
                        .clone()
                        .expect("capability recheck requires last snapshot");
                    match resolver(snapshot).await {
                        Ok(desired_target) => {
                            transition_runtime_target_with_starter(
                                &mut state,
                                desired_target,
                                &context,
                                "wayland-capability-recheck",
                                &starter,
                            )
                            .await?;
                        }
                        Err(error) => {
                            if runtime_target_is_wayland_family(state.current_target) {
                                eprintln!(
                                    "[Lifecycle] Keeping {} backend after capability recheck resolver error: {}",
                                    runtime_target_label(state.current_target),
                                    error
                                );
                            } else {
                                eprintln!(
                                    "[Lifecycle] Falling back to generic Wayland after capability recheck resolver error: {}",
                                    error
                                );
                                transition_runtime_target_with_starter(
                                    &mut state,
                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                    &context,
                                    "wayland-capability-recheck-fallback-after-resolver-error",
                                    &starter,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(crate) async fn wait_for_wayland_capability_recheck(
    enabled: bool,
    last_snapshot: Option<&LifecycleSnapshot>,
    interval: Duration,
) {
    if !enabled {
        std::future::pending::<()>().await;
        return;
    }
    let Some(snapshot) = last_snapshot else {
        std::future::pending::<()>().await;
        return;
    };
    if snapshot.session_kind != SessionKind::GraphicalWayland {
        std::future::pending::<()>().await;
        return;
    }
    tokio::time::sleep(interval).await;
}

pub(crate) async fn wait_for_backend_completion_signal(backend_finished: &mut Option<watch::Receiver<bool>>) {
    let Some(receiver) = backend_finished.as_mut() else {
        std::future::pending::<()>().await;
        return;
    };
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await;
}

pub(crate) async fn poll_finished_backend_outcome(
    state: &mut SupervisorState,
) -> Result<Option<RunOutcome>, DynError> {
    let Some(backend) = state.backend.as_mut() else {
        return Ok(None);
    };
    if !backend.is_finished() {
        return Ok(None);
    }

    let exit = backend.take_join_result().await?;
    let kind = backend.kind;
    state.backend = None;
    state.current_target = RuntimeTarget::Idle;
    match exit {
        BackendExit::Restart => Ok(Some(RunOutcome::Restart)),
        BackendExit::Exit => {
            Err(format!("[Lifecycle] backend {:?} exited unexpectedly", kind).into())
        }
    }
}
