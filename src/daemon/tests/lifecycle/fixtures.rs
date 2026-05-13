use super::*;

pub(crate) fn test_backend_context_with_gnome_setup<F>(gnome_setup_hook: F) -> BackendContext
where
    F: Fn(bool) + Send + Sync + 'static,
{
    let status_broadcaster = StatusBroadcaster::new();
    BackendContext {
        kanata: KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        ),
        handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
        status_broadcaster,
        restart_handle: RestartHandle::new(),
        pause_broadcaster: PauseBroadcaster::new(),
        runtime_environment: RuntimeEnvironmentBroadcaster::new(Environment::Unknown),
        install_gnome_extension: true,
        gnome_setup_completed: Arc::new(AtomicBool::new(false)),
        gnome_setup_hook: Arc::new(gnome_setup_hook),
        effective_dbus_name: effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
    }
}

pub(crate) fn test_backend_context() -> BackendContext {
    test_backend_context_with_gnome_setup(|_| {})
}

pub(crate) fn test_running_backend_handle(kind: BackendKind, stopped: Arc<AtomicBool>) -> BackendHandle {
    let shutdown_handle = ShutdownHandle::new();
    let mut receiver = shutdown_handle.subscribe();
    let (finished_tx, finished_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
        stopped.store(true, Ordering::SeqCst);
        let _ = finished_tx.send(true);
        Ok(BackendExit::Exit)
    });
    BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    }
}

pub(crate) fn test_finished_backend_handle(kind: BackendKind, exit: BackendExit) -> BackendHandle {
    let shutdown_handle = ShutdownHandle::new();
    let (finished_tx, finished_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        let _ = finished_tx.send(true);
        Ok(exit)
    });
    BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    }
}
