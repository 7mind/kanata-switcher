use super::*;

#[test]
#[should_panic(expected = "mapped-boom")]
fn test_expect_or_fail_fast_uses_fail_handler_for_error_results() {
    let _: u8 = expect_or_fail_fast(
        Err::<u8, _>("boom"),
        |error| format!("mapped-{}", error),
        |message| panic!("{}", message),
    );
}

#[test]
#[should_panic(expected = "stream-ended")]
fn test_expect_some_or_fail_fast_uses_fail_handler_for_none() {
    let _: u8 = expect_some_or_fail_fast(None, "stream-ended".to_string(), |message| {
        panic!("{}", message)
    });
}

#[tokio::test]
async fn test_transition_runtime_target_updates_runtime_environment() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "to-x11",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to x11 should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::X11);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "to-linux-console",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to linux console should succeed");
        assert_eq!(
            context.runtime_environment.current(),
            Environment::LinuxConsoleWithLogind
        );

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "to-idle",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to idle should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_runs_gnome_setup_on_runtime_transition() {
    with_test_timeout(async {
        let setup_calls = Arc::new(AtomicUsize::new(0));
        let setup_calls_for_hook = setup_calls.clone();
        let context = test_backend_context_with_gnome_setup(move |_| {
            setup_calls_for_hook.fetch_add(1, Ordering::SeqCst);
        });
        let mut state = SupervisorState::new();

        let starter = |kind: BackendKind, _context: BackendContext| async move {
            Ok(test_running_backend_handle(
                kind,
                Arc::new(AtomicBool::new(false)),
            ))
        };

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "to-x11",
            &starter,
        )
        .await
        .expect("transition to x11 should succeed");
        assert_eq!(setup_calls.load(Ordering::SeqCst), 0);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Gnome),
            &context,
            "to-gnome-runtime-transition",
            &starter,
        )
        .await
        .expect("transition to gnome should succeed");
        assert_eq!(setup_calls.load(Ordering::SeqCst), 1);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "to-idle",
            &starter,
        )
        .await
        .expect("transition to idle should succeed");

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Gnome),
            &context,
            "to-gnome-runtime-transition-second-time",
            &starter,
        )
        .await
        .expect("second transition to gnome should succeed");
        assert_eq!(
            setup_calls.load(Ordering::SeqCst),
            1,
            "gnome setup should be cached after first runtime setup"
        );

        stop_current_backend(&mut state, &context)
            .await
            .expect("stopping test backend should succeed");
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_noop_on_same_target() {
    with_test_timeout(async {
        let context = test_backend_context();

        let mut state = SupervisorState::new();
        transition_runtime_target(&mut state, RuntimeTarget::Idle, &context, "test-noop")
            .await
            .expect("noop transition should succeed");
        assert_eq!(state.current_target, RuntimeTarget::Idle);
        assert!(state.backend.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_no_churn_on_same_target_with_running_backend() {
    with_test_timeout(async {
        let context = test_backend_context();
        let stopped = Arc::new(AtomicBool::new(false));
        let backend = test_running_backend_handle(BackendKind::LinuxConsole, stopped.clone());

        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);
        state.backend = Some(backend);

        transition_runtime_target(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "same-target",
        )
        .await
        .expect("same-target transition should be noop");

        assert_eq!(
            state.current_target,
            RuntimeTarget::Backend(BackendKind::LinuxConsole)
        );
        assert!(state.backend.is_some());
        assert!(
            !stopped.load(Ordering::SeqCst),
            "backend should not be stopped on same-target transition"
        );

        stop_current_backend(&mut state, &context)
            .await
            .expect("cleanup stop should succeed");
        assert!(stopped.load(Ordering::SeqCst));
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_stops_old_before_starting_new() {
    with_test_timeout(async {
        let context = test_backend_context();
        let old_stopped = Arc::new(AtomicBool::new(false));
        let old_backend = test_running_backend_handle(BackendKind::X11, old_stopped.clone());

        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::X11);
        state.backend = Some(old_backend);

        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        let old_stopped_clone = old_stopped.clone();
        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "ordering-test",
            move |kind, _| {
                let started = started_clone.clone();
                let old_stopped = old_stopped_clone.clone();
                async move {
                    assert!(
                        old_stopped.load(Ordering::SeqCst),
                        "starter must run only after previous backend is fully stopped"
                    );
                    started.store(true, Ordering::SeqCst);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
        )
        .await
        .expect("transition should succeed");

        assert!(old_stopped.load(Ordering::SeqCst));
        assert!(started.load(Ordering::SeqCst));
        assert_eq!(
            state.current_target,
            RuntimeTarget::Backend(BackendKind::LinuxConsole)
        );
        assert!(state.backend.is_some());

        stop_current_backend(&mut state, &context)
            .await
            .expect("cleanup stop should succeed");
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_desktop_sequence_is_restart_equivalent() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let stopped_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));

        let starter = {
            let started_kinds = started_kinds.clone();
            let stopped_kinds = stopped_kinds.clone();
            move |kind: BackendKind, _context: BackendContext| {
                let started_kinds = started_kinds.clone();
                let stopped_kinds = stopped_kinds.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    let shutdown_handle = ShutdownHandle::new();
                    let mut receiver = shutdown_handle.subscribe();
                    let (finished_tx, finished_rx) = watch::channel(false);
                    let join_handle = tokio::spawn(async move {
                        while !*receiver.borrow() {
                            if receiver.changed().await.is_err() {
                                break;
                            }
                        }
                        stopped_kinds.lock().unwrap().push(kind);
                        let _ = finished_tx.send(true);
                        Ok(BackendExit::Exit)
                    });
                    Ok(BackendHandle {
                        kind,
                        shutdown_handle,
                        join_handle: Some(join_handle),
                        finished_rx,
                    })
                }
            }
        };

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "x11",
            &starter,
        )
        .await
        .expect("x11 transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::X11);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Wayland),
            &context,
            "wayland",
            &starter,
        )
        .await
        .expect("wayland transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Wayland);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Kde),
            &context,
            "kde",
            &starter,
        )
        .await
        .expect("kde transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Kde);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "idle",
            &starter,
        )
        .await
        .expect("idle transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);
        assert!(state.backend.is_none());

        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::X11, BackendKind::Wayland, BackendKind::Kde,]
        );
        assert_eq!(
            stopped_kinds.lock().unwrap().as_slice(),
            &[BackendKind::X11, BackendKind::Wayland, BackendKind::Kde,]
        );
    })
    .await;
}

#[tokio::test]
async fn test_stop_current_backend_with_no_backend_is_noop() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);

        stop_current_backend(&mut state, &context)
            .await
            .expect("stop with no backend should succeed");

        assert_eq!(state.current_target, RuntimeTarget::Idle);
        assert!(state.backend.is_none());
    })
    .await;
}
