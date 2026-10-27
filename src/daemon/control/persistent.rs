use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use futures_util::StreamExt;
use tokio::sync::watch;
use zbus::Connection;
use crate::focus::FocusHandler;
use crate::broadcasters::*;
use crate::kanata::KanataClient;
use crate::errors::DynError;
use crate::environ::Environment;
use super::server::*;

const DBUS_RECONNECT_DELAYS_MS: &[u64] = &[250, 1000, 2000];

pub(crate) fn dbus_reconnect_delay(attempt: usize) -> Duration {
    let index = attempt.min(DBUS_RECONNECT_DELAYS_MS.len() - 1);
    Duration::from_millis(DBUS_RECONNECT_DELAYS_MS[index])
}

pub(crate) async fn wait_for_dbus_reconnect_retry(
    reconnect_attempt: &mut usize,
    shutdown_receiver: &mut watch::Receiver<bool>,
    restart_receiver: &mut watch::Receiver<bool>,
    prefix: &str,
    error: String,
) {
    let delay = dbus_reconnect_delay(*reconnect_attempt);
    eprintln!("{}; retrying in {}ms: {}", prefix, delay.as_millis(), error);
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = shutdown_receiver.changed() => {}
        _ = restart_receiver.changed() => {}
    }
    *reconnect_attempt += 1;
}

pub(crate) struct PersistentDbusServiceGuard {
    pub(crate) task: tokio::task::JoinHandle<()>,
}

impl Drop for PersistentDbusServiceGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) fn start_persistent_dbus_service(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> PersistentDbusServiceGuard {
    start_persistent_dbus_service_with_connector(
        || async {
            Connection::session()
                .await
                .map_err(|error| -> DynError { Box::new(error) })
        },
        kanata,
        handler,
        status_broadcaster,
        restart_handle,
        pause_broadcaster,
        runtime_environment,
        shutdown_handle,
        effective_name,
    )
}

pub(crate) fn start_persistent_dbus_service_with_connector<C, CFut>(
    connector: C,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> PersistentDbusServiceGuard
where
    C: Fn() -> CFut + Send + Sync + 'static,
    CFut: std::future::Future<Output = Result<Connection, DynError>> + Send + 'static,
{
    let task = tokio::spawn(async move {
        run_persistent_dbus_service_with_connector(
            connector,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            runtime_environment,
            shutdown_handle,
            effective_name,
        )
        .await;
    });
    PersistentDbusServiceGuard { task }
}

pub(crate) async fn run_persistent_dbus_service_with_connector<C, CFut>(
    connector: C,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) where
    C: Fn() -> CFut + Send + Sync + 'static,
    CFut: std::future::Future<Output = Result<Connection, DynError>> + Send + 'static,
{
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();
    let mut reconnect_attempt = 0usize;

    loop {
        if *shutdown_receiver.borrow() || *restart_receiver.borrow() {
            return;
        }

        let connection = match connector().await {
            Ok(connection) => {
                reconnect_attempt = 0;
                connection
            }
            Err(error) => {
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Session bus unavailable",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        let registration = match register_dbus_service_with_runtime_environment(
            &connection,
            connection.clone(),
            Environment::Unknown,
            false,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            restart_handle.clone(),
            pause_broadcaster.clone(),
            Some(runtime_environment.clone()),
            &effective_name,
        )
        .await
        {
            Ok(registration) => {
                println!("[DBus] Control service registered as {}", effective_name);
                reconnect_attempt = 0;
                registration
            }
            Err(error) => {
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to register control service",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        let proxy = match zbus::fdo::DBusProxy::new(&connection).await {
            Ok(proxy) => proxy,
            Err(error) => {
                drop(registration);
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to create DBus proxy for name-loss monitoring",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };
        let mut name_lost = match proxy
            .receive_name_lost_with_args(&[(0, effective_name.as_str())])
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                drop(registration);
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to subscribe to NameLost",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        tokio::select! {
            _ = shutdown_receiver.changed() => {
                drop(registration);
            }
            _ = restart_receiver.changed() => {
                drop(registration);
            }
            signal = name_lost.next() => {
                match signal {
                    Some(signal) => {
                        match signal.args() {
                            Ok(args) => {
                                eprintln!(
                                    "[DBus] Lost well-known name {}; re-registering",
                                    args.name()
                                );
                            }
                            Err(error) => {
                                eprintln!(
                                    "[DBus] Failed to decode NameLost signal; re-registering: {}",
                                    error
                                );
                            }
                        }
                    }
                    None => {
                        eprintln!("[DBus] NameLost stream terminated; re-registering");
                    }
                }
                drop(registration);
            }
        }
    }
}
