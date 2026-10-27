use crate::control::ControlCommand;
use crate::control::client::send_control_command_with_connection;
use crate::pause::{pause_daemon, unpause_daemon};
use super::SniControl;

pub(crate) trait SniControlOps: Send + Sync {
    fn restart(&self);
    fn pause(&self);
    fn unpause(&self);
    fn quit(&self);
}

impl SniControlOps for SniControl {
    fn restart(&self) {
        println!("[SNI] Restart requested");
        match self {
            SniControl::Local(control) => {
                control.restart_handle.request();
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        &control.daemon_bus_name,
                        ControlCommand::Restart,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send restart: {}", error);
                    }
                });
                control.restart_handle.request();
            }
        }
    }

    fn pause(&self) {
        println!("[SNI] Pause requested");
        match self {
            SniControl::Local(control) => {
                pause_daemon(
                    &control.pause_broadcaster,
                    &control.handler,
                    &control.status_broadcaster,
                    &control.kanata,
                    &control.runtime_handle,
                    "via SNI",
                );
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        &control.daemon_bus_name,
                        ControlCommand::Pause,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send pause: {}", error);
                    }
                });
            }
        }
    }

    fn unpause(&self) {
        println!("[SNI] Unpause requested");
        match self {
            SniControl::Local(control) => {
                let context = control.unpause_context.clone();
                unpause_daemon(
                    context.env,
                    context.connection,
                    context.is_kde6,
                    &control.pause_broadcaster,
                    &control.handler,
                    &control.status_broadcaster,
                    &control.kanata,
                    &control.runtime_handle,
                    "via SNI",
                );
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        &control.daemon_bus_name,
                        ControlCommand::Unpause,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send unpause: {}", error);
                    }
                });
            }
        }
    }

    fn quit(&self) {
        println!("[SNI] Quit requested");
        match self {
            SniControl::Local(control) => {
                control.shutdown_handle.request();
            }
            SniControl::Dbus(control) => {
                control.shutdown_handle.request();
            }
        }
    }
}
