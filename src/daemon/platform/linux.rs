use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use clap::ArgMatches;

use crate::args::{Args, resolve_control_command, resolve_install_gnome_extension};
use crate::autostart::*;
use crate::backends::*;
use crate::broadcasters::{PauseBroadcaster, RuntimeEnvironmentBroadcaster, ShutdownHandle, StatusBroadcaster};
use crate::control::client::send_control_command;
use crate::control::ControlDispatch;
use crate::dbus_naming::*;
use crate::display_override::*;
use crate::environ::*;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::gnome_ext::*;
use crate::kanata::KanataClient;
use crate::lifecycle::*;
use crate::lifecycle::startup::*;
use crate::sni::*;
use crate::supervisor::*;

pub(crate) async fn run(
    args: Args,
    kanata: KanataClient,
    focus_handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let matches = Args::command().get_matches();

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

    let lifecycle_provider = LifecycleProvider::build(detected_env).await;
    if !lifecycle_provider.is_continuous() && detected_env == Environment::Unknown {
        eprintln!("[Error] Could not detect display environment");
        eprintln!("[Error] login1 unavailable and no startup graphical environment detected");
        std::process::exit(1);
    }

    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    let restart_handle = crate::broadcasters::RestartHandle::new();
    let runtime_handle = tokio::runtime::Handle::current();

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

    let shutdown_handle_for_signal = shutdown_handle.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("failed to install SIGINT handler");
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler");

        tokio::select! {
            _ = sigterm.recv() => eprintln!("[Signal] Received SIGTERM"),
            _ = sigint.recv() => eprintln!("[Signal] Received SIGINT"),
            _ = sighup.recv() => eprintln!("[Signal] Received SIGHUP"),
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
