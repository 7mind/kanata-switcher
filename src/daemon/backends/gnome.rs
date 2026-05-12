use std::sync::{Arc, Mutex};
use futures_util::StreamExt;
use zbus::Connection;
use crate::broadcasters::{PauseBroadcaster, RestartHandle, ShutdownHandle, StatusBroadcaster, wait_for_restart_or_shutdown};
use crate::config::WindowInfo;
use crate::constants::*;
use crate::environ::{Environment, RunOutcome};
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};
use crate::kanata::KanataClient;
use crate::backends::apply_focus_for_env;

pub(crate) async fn query_gnome_focus(
    connection: &Connection,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let reply = connection
        .call_method(
            Some(GNOME_SHELL_BUS_NAME),
            GNOME_FOCUS_OBJECT_PATH,
            Some(GNOME_FOCUS_INTERFACE),
            GNOME_FOCUS_METHOD,
            &(),
        )
        .await?;
    let (class, title): (String, String) = reply.body().deserialize()?;
    Ok(WindowInfo {
        class,
        title,
        is_native_terminal: false,
    })
}

pub(crate) async fn run_gnome(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let focus_query_connection = Connection::session().await?;
    apply_focus_for_env(
        Environment::Gnome,
        Some(&focus_query_connection),
        false,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    let signal_connection = Connection::session().await?;
    let focus_signal_subscription = subscribe_to_gnome_focus_signal(
        &signal_connection,
        kanata.clone(),
        handler.clone(),
        status_broadcaster.clone(),
        pause_broadcaster.clone(),
    )
    .await?;

    println!(
        "[GNOME] Listening for FocusChanged signals from extension at {}",
        GNOME_FOCUS_OBJECT_PATH
    );
    let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
    drop(focus_signal_subscription);
    drop(signal_connection);
    Ok(outcome)
}

/// Guard owning the GNOME FocusChanged signal subscription task. Dropping the
/// guard aborts the listener (and releases the match rule when the connection
/// is dropped).
pub(crate) struct GnomeFocusSignalSubscription {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for GnomeFocusSignalSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn subscribe_to_gnome_focus_signal(
    connection: &Connection,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
) -> Result<GnomeFocusSignalSubscription, Box<dyn std::error::Error + Send + Sync>> {
    use zbus::MatchRule;
    let match_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(GNOME_SHELL_BUS_NAME)
        .map_err(|error| format!("Invalid sender match rule: {}", error))?
        .interface(GNOME_FOCUS_INTERFACE)
        .map_err(|error| format!("Invalid interface match rule: {}", error))?
        .path(GNOME_FOCUS_OBJECT_PATH)
        .map_err(|error| format!("Invalid path match rule: {}", error))?
        .member(GNOME_FOCUS_SIGNAL)
        .map_err(|error| format!("Invalid member match rule: {}", error))?
        .build();

    let mut stream = zbus::MessageStream::for_match_rule(match_rule, connection, None).await?;
    let task = tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    eprintln!("[GNOME] FocusChanged signal error: {}", error);
                    continue;
                }
            };
            let (window_class, window_title): (String, String) =
                match message.body().deserialize() {
                    Ok(payload) => payload,
                    Err(error) => {
                        eprintln!("[GNOME] FocusChanged decode error: {}", error);
                        continue;
                    }
                };
            let win = WindowInfo {
                class: window_class,
                title: window_title,
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            if let Some(actions) = handle_focus_event(
                &handler,
                &status_broadcaster,
                &pause_broadcaster,
                &win,
                &kanata,
                &default_layer,
            )
            .await
            {
                execute_focus_actions(&kanata, actions).await;
            }
        }
    });
    Ok(GnomeFocusSignalSubscription { task })
}
