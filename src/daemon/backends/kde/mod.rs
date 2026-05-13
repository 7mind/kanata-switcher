pub(crate) mod probe;
pub(crate) mod script;
pub(crate) use probe::*;
pub(crate) use script::*;

use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;
use crate::broadcasters::{PauseBroadcaster, RestartHandle, ShutdownHandle, StatusBroadcaster};
use crate::constants::*;
use crate::environ::{Environment, RunOutcome};
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::kanata::KanataClient;
use crate::backends::{apply_focus_for_env, BackendExit, BackendRunContext, FocusBackend, map_run_outcome_to_backend_exit};

#[derive(Debug)]
pub(crate) struct KwinScriptGuard {
    connection: Connection,
    runtime_handle: tokio::runtime::Handle,
    script_path: String,
    script_obj_path: OwnedObjectPath,
    script_interface: String,
}

impl KwinScriptGuard {
    pub(crate) fn new(
        connection: Connection,
        runtime_handle: tokio::runtime::Handle,
        script_path: String,
        script_obj_path: OwnedObjectPath,
        script_interface: &str,
    ) -> Self {
        Self {
            connection,
            runtime_handle,
            script_path,
            script_obj_path,
            script_interface: script_interface.to_string(),
        }
    }
}

impl Drop for KwinScriptGuard {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let runtime_handle = self.runtime_handle.clone();
        let script_path = self.script_path.clone();
        let script_obj_path = self.script_obj_path.clone();
        let script_interface = self.script_interface.clone();

        let cleanup = async move {
            match tokio::time::timeout(
                KDE_KWIN_SCRIPT_CLEANUP_TIMEOUT,
                connection.call_method(
                    Some("org.kde.KWin"),
                    script_obj_path.clone(),
                    Some(script_interface.as_str()),
                    "stop",
                    &(),
                ),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    eprintln!("[KDE] Failed to stop KWin script during cleanup: {}", error);
                }
                Err(_) => {
                    eprintln!(
                        "[KDE] Timed out stopping KWin script during cleanup after {}ms",
                        KDE_KWIN_SCRIPT_CLEANUP_TIMEOUT.as_millis()
                    );
                }
            }

            match tokio::time::timeout(
                KDE_KWIN_SCRIPT_CLEANUP_TIMEOUT,
                connection.call_method(
                    Some("org.kde.KWin"),
                    "/Scripting",
                    Some("org.kde.kwin.Scripting"),
                    "unloadScript",
                    &(&script_path,),
                ),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    eprintln!(
                        "[KDE] Failed to unload KWin script during cleanup: {}",
                        error
                    );
                }
                Err(_) => {
                    eprintln!(
                        "[KDE] Timed out unloading KWin script during cleanup after {}ms",
                        KDE_KWIN_SCRIPT_CLEANUP_TIMEOUT.as_millis()
                    );
                }
            }
        };

        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| {
                runtime_handle.block_on(cleanup);
            });
        } else {
            runtime_handle.block_on(cleanup);
        }

        if let Err(error) = fs::remove_file(&self.script_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "[KDE] Failed to remove KWin script file during cleanup: {}",
                    error
                );
            }
        }
    }
}

pub(crate) async fn run_kde(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    let focus_query_connection = Connection::session().await?;
    let runtime_handle = tokio::runtime::Handle::current();
    let is_kde6 = resolve_kde_runtime_query_mode_with_retry(&focus_query_connection).await?;

    apply_focus_for_env(
        Environment::Kde,
        Some(&focus_query_connection),
        is_kde6,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    // Inject KWin script (DBus service is ready to receive calls)
    let api = if is_kde6 {
        "windowActivated"
    } else {
        "clientActivated"
    };
    let active_window = if is_kde6 {
        "activeWindow"
    } else {
        "activeClient"
    };
    let kwin_script = build_kde_focus_push_script(&effective_name, api, active_window);

    let script_path = kwin_runtime_script_path();
    fs::write(&script_path, &kwin_script)?;

    for _ in 0..5 {
        let result = connection
            .call_method(
                Some("org.kde.KWin"),
                KDE_KWIN_SCRIPTING_PATH,
                Some(KDE_KWIN_SCRIPTING_INTERFACE),
                "loadScript",
                &(&script_path,),
            )
            .await;

        if result.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let _ = connection
        .call_method(
            Some("org.kde.KWin"),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "unloadScript",
            &(&script_path,),
        )
        .await;

    let load_result = connection
        .call_method(
            Some("org.kde.KWin"),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "loadScript",
            &(&script_path,),
        )
        .await?;

    let script_num: i32 = load_result.body().deserialize()?;

    let script_obj_path_str = if is_kde6 {
        format!("/Scripting/Script{}", script_num)
    } else {
        format!("/{}", script_num)
    };

    let script_interface = if is_kde6 {
        "org.kde.kwin.Script"
    } else {
        KDE_KWIN_SCRIPTING_INTERFACE
    };

    let script_obj_path: OwnedObjectPath = script_obj_path_str.as_str().try_into()?;

    let _kwin_script_guard = KwinScriptGuard::new(
        connection.clone(),
        runtime_handle.clone(),
        script_path.clone(),
        script_obj_path.clone(),
        script_interface,
    );

    connection
        .call_method(
            Some("org.kde.KWin"),
            script_obj_path,
            Some(script_interface),
            "run",
            &(),
        )
        .await?;

    println!("[KDE] KWin script injected, listening for window focus events...");

    let outcome = crate::broadcasters::wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
    Ok(outcome)
}

pub(crate) struct KdeBackend;

impl FocusBackend for KdeBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            let outcome = run_kde(
                ctx.kanata,
                ctx.focus_handler,
                ctx.status_broadcaster,
                ctx.restart_handle,
                ctx.pause_broadcaster,
                ctx.shutdown_handle,
                ctx.effective_bus_name,
            )
            .await?;
            Ok(map_run_outcome_to_backend_exit(outcome))
        })
    }
}
