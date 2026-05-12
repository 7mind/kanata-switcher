use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::Connection;
use crate::kanata::KanataClient;
use crate::focus::FocusHandler;
use crate::broadcasters::{StatusBroadcaster, PauseBroadcaster, RestartHandle, ShutdownHandle, RuntimeEnvironmentBroadcaster};
use crate::args::TrayFocusOnly;
use crate::environ::Environment;
use crate::pause::local_sni_unpause_context;
use super::{SniControl, SniControlMode, SniRuntimeTransitionPlan, SniRuntimeWakeReason};
use super::{sni_control_mode_for_environment, plan_sni_runtime_transition, wait_for_sni_runtime_wake_with_delay};
use super::control_local::SniLocalControl;
use super::control_dbus::SniDbusControl;
use super::indicator::{SniIndicatorRuntimeHandle, start_sni_indicator};

pub(crate) const SNI_RUNTIME_RETRY_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct SniGuard {
    handle: Arc<Mutex<Option<SniIndicatorRuntimeHandle>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SniGuard {
    pub(crate) fn disabled() -> Self {
        Self {
            handle: Arc::new(Mutex::new(None)),
            task: None,
        }
    }

    pub(crate) fn runtime_managed(
        runtime_environment: RuntimeEnvironmentBroadcaster,
        runtime_handle: tokio::runtime::Handle,
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        status_broadcaster: StatusBroadcaster,
        pause_broadcaster: PauseBroadcaster,
        restart_handle: RestartHandle,
        shutdown_handle: ShutdownHandle,
        indicator_focus_only: Option<TrayFocusOnly>,
        daemon_bus_name: String,
    ) -> Self {
        Self::runtime_managed_with_builder(
            runtime_environment,
            runtime_handle,
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            shutdown_handle,
            indicator_focus_only,
            SNI_RUNTIME_RETRY_INTERVAL,
            build_sni_control_for_mode,
            daemon_bus_name,
        )
    }

    pub(crate) fn runtime_managed_with_builder<B, BFut>(
        runtime_environment: RuntimeEnvironmentBroadcaster,
        runtime_handle: tokio::runtime::Handle,
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        status_broadcaster: StatusBroadcaster,
        pause_broadcaster: PauseBroadcaster,
        restart_handle: RestartHandle,
        shutdown_handle: ShutdownHandle,
        indicator_focus_only: Option<TrayFocusOnly>,
        retry_delay: Duration,
        control_builder: B,
        daemon_bus_name: String,
    ) -> Self
    where
        B: Fn(
                SniControlMode,
                tokio::runtime::Handle,
                KanataClient,
                Arc<Mutex<FocusHandler>>,
                StatusBroadcaster,
                PauseBroadcaster,
                RestartHandle,
                ShutdownHandle,
                Environment,
                String,
            ) -> BFut
            + Send
            + Sync
            + 'static,
        BFut: std::future::Future<Output = Option<SniControl>> + Send + 'static,
    {
        let shared_handle: Arc<Mutex<Option<SniIndicatorRuntimeHandle>>> =
            Arc::new(Mutex::new(None));
        let task_handle_store = shared_handle.clone();
        let mut env_receiver = runtime_environment.subscribe();
        let task = tokio::spawn(async move {
            let mut active_mode: Option<SniControlMode> = None;
            let mut active_env: Option<Environment> = None;
            loop {
                let env = *env_receiver.borrow();
                match plan_sni_runtime_transition(active_mode, active_env, env) {
                    SniRuntimeTransitionPlan::Keep => {}
                    SniRuntimeTransitionPlan::Stop => {
                        if let Some(handle) = task_handle_store.lock().unwrap().take() {
                            println!("[SNI] Shutting down indicator");
                            drop(handle);
                        }
                        active_mode = None;
                        active_env = None;
                    }
                    SniRuntimeTransitionPlan::Start(mode)
                    | SniRuntimeTransitionPlan::Restart(mode) => {
                        if let Some(handle) = task_handle_store.lock().unwrap().take() {
                            println!("[SNI] Shutting down indicator");
                            drop(handle);
                        }
                        active_mode = None;
                        active_env = None;
                        let control = control_builder(
                            mode,
                            runtime_handle.clone(),
                            kanata.clone(),
                            handler.clone(),
                            status_broadcaster.clone(),
                            pause_broadcaster.clone(),
                            restart_handle.clone(),
                            shutdown_handle.clone(),
                            env,
                            daemon_bus_name.clone(),
                        )
                        .await;
                        if let Some(control) = control {
                            let handle = start_sni_indicator(
                                control,
                                status_broadcaster.clone(),
                                pause_broadcaster.clone(),
                                indicator_focus_only,
                            );
                            *task_handle_store.lock().unwrap() = handle;
                            active_mode = Some(mode);
                            active_env = Some(env);
                        } else {
                            eprintln!(
                                "[SNI] Failed to initialize {:?} control; retrying in {}ms unless environment changes",
                                mode,
                                retry_delay.as_millis()
                            );
                        }
                    }
                }

                let should_retry_start =
                    active_mode.is_none() && sni_control_mode_for_environment(env).is_some();
                if matches!(
                    wait_for_sni_runtime_wake_with_delay(
                        &mut env_receiver,
                        should_retry_start,
                        retry_delay,
                    )
                    .await,
                    SniRuntimeWakeReason::ChannelClosed
                ) {
                    break;
                }
            }
        });
        Self {
            handle: shared_handle,
            task: Some(task),
        }
    }
}

impl Drop for SniGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(handle) = self.handle.lock().unwrap().take() {
            println!("[SNI] Shutting down indicator");
            drop(handle);
        }
    }
}

pub(crate) async fn build_sni_control_for_mode(
    mode: SniControlMode,
    runtime_handle: tokio::runtime::Handle,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    control_environment: Environment,
    daemon_bus_name: String,
) -> Option<SniControl> {
    match mode {
        SniControlMode::Local => Some(SniControl::Local(SniLocalControl {
            runtime_handle,
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            shutdown_handle,
            unpause_context: local_sni_unpause_context(control_environment),
        })),
        SniControlMode::Dbus => match Connection::session().await {
            Ok(connection) => Some(SniControl::Dbus(SniDbusControl {
                runtime_handle,
                connection,
                restart_handle,
                shutdown_handle,
                daemon_bus_name,
            })),
            Err(error) => {
                eprintln!("[SNI] Failed to connect to session bus: {}", error);
                None
            }
        },
    }
}
