use std::sync::Arc;
use std::sync::Mutex;
use zbus::Connection;
use zbus::object_server::SignalEmitter;
use crate::environ::Environment;
use crate::config::WindowInfo;
use crate::focus::FocusHandler;
use crate::broadcasters::*;
use crate::kanata::KanataClient;
use crate::pause::*;
use crate::focus_pipeline::*;
use crate::errors::DynError;
use crate::constants::DBUS_PATH;
use crate::backends::kde::probe::*;

#[derive(Debug)]
pub(crate) struct DbusWindowFocusService {
    pub(crate) kanata: KanataClient,
    pub(crate) handler: Arc<Mutex<FocusHandler>>,
    pub(crate) runtime_handle: tokio::runtime::Handle,
    pub(crate) status_broadcaster: StatusBroadcaster,
    pub(crate) restart_handle: RestartHandle,
    pub(crate) pause_broadcaster: PauseBroadcaster,
    pub(crate) env: Environment,
    pub(crate) focus_query_connection: Connection,
    pub(crate) is_kde6: bool,
    pub(crate) runtime_environment: Option<RuntimeEnvironmentBroadcaster>,
}

#[zbus::interface(name = "com.github.kanata.Switcher")]
impl DbusWindowFocusService {
    async fn window_focus(&self, window_class: &str, window_title: &str) {
        let win = WindowInfo {
            class: window_class.to_string(),
            title: window_title.to_string(),
            is_native_terminal: false,
        };

        if self.pause_broadcaster.is_paused() {
            return;
        }

        let default_layer = self
            .runtime_handle
            .block_on(async { self.kanata.default_layer().await })
            .unwrap_or_default();

        let actions = self.runtime_handle.block_on(async {
            update_status_for_focus(
                &self.handler,
                &self.status_broadcaster,
                &win,
                &self.kanata,
                &default_layer,
            )
            .await
        });

        if let Some(actions) = actions {
            let kanata = self.kanata.clone();
            self.runtime_handle
                .block_on(async { execute_focus_actions(&kanata, actions).await });
        }
    }

    async fn get_status(&self) -> (String, Vec<String>, String) {
        let snapshot = self.status_broadcaster.snapshot();
        (
            snapshot.layer,
            snapshot.virtual_keys,
            snapshot.layer_source.as_str().to_string(),
        )
    }

    async fn get_paused(&self) -> bool {
        self.pause_broadcaster.is_paused()
    }

    #[zbus(signal)]
    async fn status_changed(
        signal_emitter: &SignalEmitter<'_>,
        layer: &str,
        virtual_keys: &[&str],
        source: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn paused_changed(signal_emitter: &SignalEmitter<'_>, paused: bool) -> zbus::Result<()>;

    async fn restart(&self) {
        println!("[Restart] Restart requested via DBus");
        self.restart_handle.request();
    }

    async fn pause(&self) {
        pause_daemon(
            &self.pause_broadcaster,
            &self.handler,
            &self.status_broadcaster,
            &self.kanata,
            &self.runtime_handle,
            "via DBus",
        );
    }

    async fn unpause(&self) {
        let context = match &self.runtime_environment {
            Some(runtime_environment) => resolve_runtime_unpause_context(runtime_environment).await,
            None => UnpauseContext {
                env: self.env,
                connection: Some(self.focus_query_connection.clone()),
                is_kde6: self.is_kde6,
            },
        };
        unpause_daemon(
            context.env,
            context.connection,
            context.is_kde6,
            &self.pause_broadcaster,
            &self.handler,
            &self.status_broadcaster,
            &self.kanata,
            &self.runtime_handle,
            "via DBus",
        );
    }
}

pub(crate) async fn resolve_runtime_unpause_context(
    runtime_environment: &RuntimeEnvironmentBroadcaster,
) -> UnpauseContext {
    let env = runtime_environment.current();
    let connection = if environment_requires_focus_query_connection(env) {
        Some(Connection::session().await.unwrap_or_else(|error| {
            panic!(
                "[DBus] Failed to connect to session bus for unpause focus query: {}",
                error
            )
        }))
    } else {
        None
    };
    let is_kde6 = if env == Environment::Kde {
        let connection_ref = connection
            .as_ref()
            .expect("KDE runtime unpause context requires session connection");
        resolve_kde_runtime_query_mode_with_retry(connection_ref)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "[KDE] Failed to resolve runtime query mode for unpause: {}",
                    error
                )
            })
    } else {
        false
    };
    UnpauseContext {
        env,
        connection,
        is_kde6,
    }
}

pub(crate) struct DbusServiceRegistration {
    pub(crate) _connection: Connection,
    pub(crate) status_signal_task: tokio::task::JoinHandle<()>,
    pub(crate) pause_signal_task: tokio::task::JoinHandle<()>,
}

impl Drop for DbusServiceRegistration {
    fn drop(&mut self) {
        self.status_signal_task.abort();
        self.pause_signal_task.abort();
    }
}

#[cfg(test)]
pub(crate) async fn register_dbus_service(
    connection: &Connection,
    focus_query_connection: Connection,
    env: Environment,
    is_kde6: bool,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    effective_name: &str,
) -> Result<DbusServiceRegistration, DynError> {
    register_dbus_service_with_runtime_environment(
        connection,
        focus_query_connection,
        env,
        is_kde6,
        kanata,
        handler,
        status_broadcaster,
        restart_handle,
        pause_broadcaster,
        None,
        effective_name,
    )
    .await
}

pub(crate) async fn register_dbus_service_with_runtime_environment(
    connection: &Connection,
    focus_query_connection: Connection,
    env: Environment,
    is_kde6: bool,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: Option<RuntimeEnvironmentBroadcaster>,
    effective_name: &str,
) -> Result<DbusServiceRegistration, DynError> {
    let service = DbusWindowFocusService {
        kanata,
        handler,
        runtime_handle: tokio::runtime::Handle::current(),
        status_broadcaster: status_broadcaster.clone(),
        restart_handle,
        pause_broadcaster: pause_broadcaster.clone(),
        env,
        focus_query_connection,
        is_kde6,
        runtime_environment,
    };

    connection.object_server().at(DBUS_PATH, service).await?;

    connection.request_name(effective_name).await?;

    let mut receiver = status_broadcaster.subscribe();
    let signal_emitter = SignalEmitter::new(connection, DBUS_PATH)?.into_owned();
    let initial_status = status_broadcaster.snapshot();
    let initial_virtual_keys: Vec<&str> = initial_status
        .virtual_keys
        .iter()
        .map(|vk| vk.as_str())
        .collect();
    DbusWindowFocusService::status_changed(
        &signal_emitter,
        &initial_status.layer,
        &initial_virtual_keys,
        initial_status.layer_source.as_str(),
    )
    .await?;
    let signal_emitter_task = signal_emitter.clone();
    let status_signal_task = tokio::spawn(async move {
        let mut last = receiver.borrow().clone();
        loop {
            if receiver.changed().await.is_err() {
                break;
            }
            let current = receiver.borrow().clone();
            if current != last {
                let virtual_keys: Vec<&str> =
                    current.virtual_keys.iter().map(|vk| vk.as_str()).collect();
                let _ = DbusWindowFocusService::status_changed(
                    &signal_emitter_task,
                    &current.layer,
                    &virtual_keys,
                    current.layer_source.as_str(),
                )
                .await;
                last = current;
            }
        }
    });

    let mut pause_receiver = pause_broadcaster.subscribe();
    let pause_emitter = signal_emitter.clone();
    DbusWindowFocusService::paused_changed(&pause_emitter, pause_broadcaster.is_paused()).await?;
    let pause_signal_task = tokio::spawn(async move {
        let mut last = *pause_receiver.borrow();
        loop {
            if pause_receiver.changed().await.is_err() {
                break;
            }
            let current = *pause_receiver.borrow();
            if current != last {
                let _ = DbusWindowFocusService::paused_changed(&pause_emitter, current).await;
                last = current;
            }
        }
    });

    Ok(DbusServiceRegistration {
        _connection: connection.clone(),
        status_signal_task,
        pause_signal_task,
    })
}
