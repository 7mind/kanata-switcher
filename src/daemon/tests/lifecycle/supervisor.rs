use super::*;

#[tokio::test]
async fn test_run_lifecycle_supervisor_restart_after_startup_provider_exhausted() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle.clone(),
            shutdown_handle,
        ));

        tokio::task::yield_now().await;
        restart_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_wins_when_both_pre_set() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        shutdown_handle.request();
        restart_handle.request();

        let outcome = run_lifecycle_supervisor(provider, context, restart_handle, shutdown_handle)
            .await
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_after_startup_provider_exhausted() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
        ));

        tokio::task::yield_now().await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_handles_shutdown_before_first_logind_snapshot() {
    with_test_timeout(async {
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let (_sender, receiver) = mpsc::unbounded_channel();
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });

        shutdown_handle.request();
        let outcome =
            run_lifecycle_supervisor(provider, context, restart_handle, shutdown_handle).await;
        assert_eq!(outcome.unwrap(), RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_poll_finished_backend_outcome_returns_restart() {
    with_test_timeout(async {
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);
        state.backend = Some(test_finished_backend_handle(
            BackendKind::LinuxConsole,
            BackendExit::Restart,
        ));
        for _ in 0..10 {
            if state
                .backend
                .as_ref()
                .expect("backend should be set")
                .is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let outcome = poll_finished_backend_outcome(&mut state)
            .await
            .expect("finished backend poll should succeed");
        assert_eq!(outcome, Some(RunOutcome::Restart));
        assert!(state.backend.is_none());
        assert_eq!(state.current_target, RuntimeTarget::Idle);
    })
    .await;
}

#[tokio::test]
async fn test_poll_finished_backend_outcome_errors_on_unexpected_exit() {
    with_test_timeout(async {
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::X11);
        state.backend = Some(test_finished_backend_handle(
            BackendKind::X11,
            BackendExit::Exit,
        ));
        for _ in 0..10 {
            if state
                .backend
                .as_ref()
                .expect("backend should be set")
                .is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let result = poll_finished_backend_outcome(&mut state).await;
        assert!(
            result.is_err(),
            "unexpected backend exit should be an error"
        );
        let message = result.err().expect("error expected").to_string();
        assert!(
            message.contains("exited unexpectedly"),
            "unexpected exit error should mention regression context"
        );
        assert!(state.backend.is_none());
        assert_eq!(state.current_target, RuntimeTarget::Idle);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_wins_race_while_backend_running() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "tty".to_string(),
                session_kind: SessionKind::NativeTerminal,
            })
            .expect("snapshot send should succeed");
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle.clone(),
            shutdown_handle.clone(),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        restart_handle.request();
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_wakes_on_backend_completion_after_provider_exhausts() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "tty".to_string(),
                session_kind: SessionKind::NativeTerminal,
            })
            .expect("snapshot send should succeed");
        drop(sender);
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let outcome = run_lifecycle_supervisor_with_starter(
            provider,
            context,
            restart_handle,
            shutdown_handle,
            |kind, _context| async move {
                let shutdown_handle = ShutdownHandle::new();
                let (finished_tx, finished_rx) = watch::channel(false);
                let join_handle = tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let _ = finished_tx.send(true);
                    Ok(BackendExit::Restart)
                });

                Ok(BackendHandle {
                    kind,
                    shutdown_handle,
                    join_handle: Some(join_handle),
                    finished_rx,
                })
            },
        )
        .await
        .expect("supervisor should observe backend completion");

        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_rechecks_wayland_capabilities_without_new_snapshots() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(kinds.first(), Some(&BackendKind::Wayland));
        assert!(
            kinds.contains(&BackendKind::Gnome),
            "capability recheck should promote from generic wayland to gnome"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 2,
            "resolver should be called again without new lifecycle snapshots"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_skips_wayland_capability_rechecks() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland]
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_selects_gnome_without_focus_readiness_gate() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let setup_calls = Arc::new(AtomicUsize::new(0));
        let setup_calls_for_hook = setup_calls.clone();
        let context = test_backend_context_with_gnome_setup(move |_| {
            setup_calls_for_hook.fetch_add(1, Ordering::SeqCst);
        });
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| async move {
                Ok(resolve_runtime_target(
                    SessionKind::GraphicalWayland,
                    DesktopCapabilities {
                        gnome_owner: true,
                        kde_owner: false,
                    },
                ))
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Gnome]
        );
        assert_eq!(
            setup_calls.load(Ordering::SeqCst),
            1,
            "startup-only mode must run GNOME setup before starting GNOME backend"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_falls_back_to_generic_wayland_on_initial_resolver_error()
 {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    resolver_calls.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("transient startup wayland resolver failure").into())
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("startup wayland resolver errors should not terminate startup provider");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland],
            "startup mode should fall back to generic wayland on transient wayland resolver errors"
        );
        assert_eq!(
            resolver_calls.load(Ordering::SeqCst),
            1,
            "startup provider should still be one-shot after fallback"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_non_wayland_resolver_error_is_fatal() {
    with_test_timeout(async {
        let provider = LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::X11));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let result = run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle,
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| async move {
                Err(std::io::Error::other("startup resolver failure").into())
            },
            std::time::Duration::from_millis(20),
        )
        .await;

        assert!(
            result.is_err(),
            "non-wayland startup resolver failures should still fail fast"
        );
        let error = result.err().expect("error expected").to_string();
        assert!(
            error.contains("Startup lifecycle target resolution failed"),
            "error should explain startup-only resolution failure"
        );
        assert!(
            started_kinds.lock().unwrap().is_empty(),
            "no backend should start when startup resolver fails"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_recovers_after_transient_wayland_resolver_error() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else if call_index == 1 {
                        Err(std::io::Error::other("transient capability probe failure").into())
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should remain alive after transient resolver error");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(kinds.first(), Some(&BackendKind::Wayland));
        assert!(
            kinds.contains(&BackendKind::Gnome),
            "supervisor should recover and transition once resolver succeeds again"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 3,
            "resolver should continue running after transient failures"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_continuous_wayland_resolver_error_falls_back_to_generic_wayland()
 {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);

        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    resolver_calls.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("continuous wayland resolver failure").into())
                }
            },
            std::time::Duration::from_secs(5),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("continuous resolver failure should still fall back to wayland");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland],
            "continuous wayland resolver failure should trigger generic wayland fallback immediately"
        );
        assert_eq!(
            resolver_calls.load(Ordering::SeqCst),
            1,
            "fallback should happen on snapshot resolver error without waiting for periodic recheck"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_continuous_wayland_resolver_error_keeps_active_gnome_backend()
 {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);

        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    } else {
                        Err(std::io::Error::other("transient wayland capability probe failure").into())
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("transient resolver failures should not tear down active GNOME backend");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(
            kinds.as_slice(),
            &[BackendKind::Gnome],
            "continuous resolver errors should keep active wayland-family backend instead of downgrading to generic wayland"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 2,
            "resolver should keep retrying after transient failures"
        );
    })
    .await;
}
