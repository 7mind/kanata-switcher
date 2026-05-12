use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot, watch};
use uuid::Uuid;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

mod constants;
mod errors;
mod environ;
mod dbus_naming;
mod config;
mod focus;
mod args;
mod autostart;
mod broadcasters;
mod kanata;
mod control;
mod pause;
mod focus_pipeline;
mod lifecycle;
mod display_override;
mod supervisor;
mod backends;
mod sni;
mod gnome_ext;

use constants::*;
use errors::DynError;
use environ::*;
use dbus_naming::*;
use config::*;
use focus::*;
use args::*;
use autostart::*;
use broadcasters::*;
use kanata::*;
use control::*;
use control::client::*;
use pause::*;
use focus_pipeline::*;
use lifecycle::*;
use lifecycle::logind::*;
use lifecycle::startup::*;
use display_override::*;
use supervisor::*;
use backends::*;
use backends::wayland::*;
use sni::*;
use gnome_ext::*;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::{constants::*, errors::*, environ::*, dbus_naming::*, config::*, focus::*, args::*, autostart::*, broadcasters::*, kanata::*, control::*, control::client::*, control::server::*, control::persistent::*, pause::*, focus_pipeline::*, lifecycle::*, lifecycle::logind::*, lifecycle::startup::*, display_override::*, supervisor::*, supervisor::capabilities::*, backends::*, backends::gnome::*, backends::kde::*, backends::kde::script::*, backends::kde::probe::*, backends::wayland::*, sni::*, sni::settings::*, sni::state::*, sni::indicator::*, sni::control_local::*, sni::control_dbus::*, sni::control_ops::*, sni::guard::*, gnome_ext::*, gnome_ext::detection::*, gnome_ext::install::*};

// === Main ===

#[tokio::main]
async fn main() {
    loop {
        match run_once().await {
            Ok(RunOutcome::Restart) => {
                println!("[Restart] Restarting daemon");
            }
            Ok(RunOutcome::Exit) => break,
            Err(e) => {
                eprintln!("[Fatal] {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_once() -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    if args.install_autostart {
        install_autostart_desktop(&matches, &args)?;
        return Ok(RunOutcome::Exit);
    }
    if args.uninstall_autostart {
        uninstall_autostart_desktop()?;
        return Ok(RunOutcome::Exit);
    }
    if let Some(command) = resolve_control_command(&args) {
        let dispatch = match args.dbus_suffix.as_deref() {
            Some(suffix) => ControlDispatch::Unicast {
                bus_name: effective_dbus_name(suffix),
            },
            None => ControlDispatch::Broadcast,
        };
        send_control_command(command, dispatch).await?;
        return Ok(RunOutcome::Exit);
    }
    let effective_name = effective_dbus_name(&resolve_dbus_suffix(
        args.dbus_suffix.as_deref(),
        &args.host,
        args.port,
    )?);
    println!("[DBus] Using bus name: {}", effective_name);

    let install_gnome_extension = resolve_install_gnome_extension(&matches);

    let detected_env = detect_environment();
    println!("[Init] Detected environment: {}", detected_env.as_str());

    let config = load_config(args.config.as_deref());
    if config.rules.is_empty() && config.native_terminal_rule.is_none() {
        eprintln!("[Config] Error: No rules found in config file");
        eprintln!();
        eprintln!("Example config (~/.config/kanata/kanata-switcher.json):");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    let quiet_focus = args.quiet || args.quiet_focus;
    let status_broadcaster = StatusBroadcaster::new();
    let restart_handle = RestartHandle::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let shutdown_handle = ShutdownHandle::new();
    let runtime_handle = tokio::runtime::Handle::current();
    let kanata = KanataClient::new(
        &args.host,
        args.port,
        config.default_layer,
        args.quiet,
        status_broadcaster.clone(),
    );
    kanata.connect_with_retry().await;

    let focus_handler = Arc::new(Mutex::new(FocusHandler::new(
        config.rules.clone(),
        config.native_terminal_rule.clone(),
        quiet_focus,
    )));

    let lifecycle_provider = LifecycleProvider::build(detected_env).await;
    if !lifecycle_provider.is_continuous() && detected_env == Environment::Unknown {
        eprintln!("[Error] Could not detect display environment");
        eprintln!("[Error] login1 unavailable and no startup graphical environment detected");
        std::process::exit(1);
    }

    // Create shutdown guard - will switch to default layer when dropped
    let _shutdown_guard = ShutdownGuard::new(kanata.clone());
    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    let _persistent_dbus_service_guard = start_persistent_dbus_service(
        kanata.clone(),
        focus_handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
        runtime_environment.clone(),
        shutdown_handle.clone(),
        effective_name.clone(),
    );

    // Set up signal handlers
    let shutdown_handle_for_signal = shutdown_handle.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("failed to install SIGINT handler");
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler");

        tokio::select! {
            _ = sigterm.recv() => {
                eprintln!("[Signal] Received SIGTERM");
            }
            _ = sigint.recv() => {
                eprintln!("[Signal] Received SIGINT");
            }
            _ = sighup.recv() => {
                eprintln!("[Signal] Received SIGHUP");
            }
        }

        shutdown_handle_for_signal.request();
    });

    let enable_indicator = !args.no_indicator;
    if args.no_indicator {
        println!("[SNI] Indicator disabled via --no-indicator");
    }

    let _sni_guard = if enable_indicator {
        SniGuard::runtime_managed(
            runtime_environment.clone(),
            runtime_handle.clone(),
            kanata.clone(),
            focus_handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
            restart_handle.clone(),
            shutdown_handle.clone(),
            args.indicator_focus_only,
            effective_name.clone(),
        )
    } else {
        SniGuard::disabled()
    };

    let backend_context = BackendContext {
        kanata,
        handler: focus_handler,
        status_broadcaster,
        restart_handle: restart_handle.clone(),
        pause_broadcaster,
        runtime_environment,
        install_gnome_extension,
        gnome_setup_completed: Arc::new(AtomicBool::new(false)),
        gnome_setup_hook: Arc::new(setup_gnome_extension),
        effective_dbus_name: effective_name,
    };

    run_lifecycle_supervisor(
        lifecycle_provider,
        backend_context,
        restart_handle,
        shutdown_handle,
    )
    .await
}

// === Tests ===

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;
