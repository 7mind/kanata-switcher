use std::time::Duration;
use zbus::Connection;
use crate::control::{ControlCommand, ControlDispatch};
use crate::constants::{DBUS_PATH, DBUS_INTERFACE};
use crate::dbus_naming::is_daemon_bus_name;

pub(crate) const BROADCAST_PER_CALL_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) async fn send_control_command(
    command: ControlCommand,
    dispatch: ControlDispatch,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    match dispatch {
        ControlDispatch::Unicast { bus_name } => {
            send_control_command_with_connection(&connection, &bus_name, command).await?;
            println!("[Control] {}: {} ok", bus_name, command.label());
            Ok(())
        }
        ControlDispatch::Broadcast => {
            let report = send_control_command_broadcast(&connection, command).await?;
            for entry in &report.results {
                match &entry.outcome {
                    Ok(()) => println!("[Control] {}: {} ok", entry.bus_name, command.label()),
                    Err(error) => eprintln!(
                        "[Control] {}: {} failed: {}",
                        entry.bus_name,
                        command.label(),
                        error
                    ),
                }
            }
            if report.results.iter().all(|entry| entry.outcome.is_err()) {
                return Err(format!(
                    "All daemons failed to respond to {}",
                    command.label()
                )
                .into());
            }
            Ok(())
        }
    }
}

/// Send a control command to a specific daemon by bus name. Used by both the
/// unicast control CLI path and the broadcast fan-out (per-target).
pub(crate) async fn send_control_command_with_connection(
    connection: &Connection,
    bus_name: &str,
    command: ControlCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    connection
        .call_method(
            Some(bus_name),
            DBUS_PATH,
            Some(DBUS_INTERFACE),
            command.dbus_method(),
            &(),
        )
        .await?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct BroadcastEntryReport {
    pub(crate) bus_name: String,
    pub(crate) outcome: Result<(), Box<dyn std::error::Error + Send + Sync>>,
}

#[derive(Debug)]
pub(crate) struct BroadcastReport {
    pub(crate) results: Vec<BroadcastEntryReport>,
}

/// Enumerate every well-known bus name in the `instances.*` subtree.
pub(crate) async fn enumerate_daemon_names(
    connection: &Connection,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let names = dbus.list_names().await?;
    let mut filtered: Vec<String> = names
        .into_iter()
        .map(|name| name.as_str().to_string())
        .filter(|name| is_daemon_bus_name(name))
        .collect();
    filtered.sort();
    filtered.dedup();
    Ok(filtered)
}

pub(crate) async fn send_control_command_broadcast(
    connection: &Connection,
    command: ControlCommand,
) -> Result<BroadcastReport, Box<dyn std::error::Error + Send + Sync>> {
    let names = enumerate_daemon_names(connection).await?;
    if names.is_empty() {
        return Err(
            "No daemons running (no owners under com.github.kanata.Switcher.instances.*)".into(),
        );
    }
    // Dispatch in parallel so one hanging daemon doesn't extend total broadcast
    // time by `N * per_call_timeout`. Per-call `tokio::time::timeout` still
    // bounds individual call time; the futures share the same connection
    // (cheap to clone — `zbus::Connection` is internally `Arc`-backed).
    let futures = names.iter().cloned().map(|name| {
        let connection = connection.clone();
        async move {
            let outcome = tokio::time::timeout(
                BROADCAST_PER_CALL_TIMEOUT,
                send_control_command_with_connection(&connection, &name, command),
            )
            .await;
            let outcome: Result<(), Box<dyn std::error::Error + Send + Sync>> = match outcome {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(format!(
                    "timed out after {}ms",
                    BROADCAST_PER_CALL_TIMEOUT.as_millis()
                )
                .into()),
            };
            BroadcastEntryReport {
                bus_name: name,
                outcome,
            }
        }
    });
    let results = futures_util::future::join_all(futures).await;
    Ok(BroadcastReport { results })
}
