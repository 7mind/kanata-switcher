use super::*;

#[test]
fn test_sni_control_mode_tracks_runtime_environment_changes() {
    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        None
    );

    runtime_environment.set_current(Environment::Wayland);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Local)
    );

    runtime_environment.set_current(Environment::X11);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Local)
    );

    runtime_environment.set_current(Environment::Kde);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Dbus)
    );

    runtime_environment.set_current(Environment::Gnome);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        None
    );
}

#[test]
fn test_plan_sni_runtime_transition_restarts_on_environment_change() {
    assert_eq!(
        plan_sni_runtime_transition(None, None, Environment::Unknown),
        SniRuntimeTransitionPlan::Keep
    );
    assert_eq!(
        plan_sni_runtime_transition(None, None, Environment::Wayland),
        SniRuntimeTransitionPlan::Start(SniControlMode::Local)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::X11),
            Environment::Wayland
        ),
        SniRuntimeTransitionPlan::Restart(SniControlMode::Local)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::Wayland),
            Environment::Wayland
        ),
        SniRuntimeTransitionPlan::Keep
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::Wayland),
            Environment::Kde
        ),
        SniRuntimeTransitionPlan::Restart(SniControlMode::Dbus)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Dbus),
            Some(Environment::Kde),
            Environment::Gnome
        ),
        SniRuntimeTransitionPlan::Stop
    );
}

#[tokio::test]
async fn test_sni_local_control_quit_triggers_shutdown_handle() {
    with_test_timeout(async {
        let status_broadcaster = StatusBroadcaster::new();
        let shutdown_handle = ShutdownHandle::new();
        let mut shutdown_receiver = shutdown_handle.subscribe();
        assert!(!*shutdown_receiver.borrow());

        let control = SniLocalControl {
            runtime_handle: tokio::runtime::Handle::current(),
            kanata: KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone()),
            handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
            status_broadcaster,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle,
            unpause_context: local_sni_unpause_context(Environment::Wayland),
        };
        let control = SniControl::Local(control);

        control.quit();

        assert!(
            *shutdown_receiver.borrow_and_update(),
            "SNI Local quit must trigger the daemon shutdown handle"
        );
    })
    .await;
}

#[tokio::test]
async fn test_sni_local_control_unpause_uses_creation_environment_during_transition_race() {
    with_test_timeout(async {
        let status_broadcaster = StatusBroadcaster::new();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Wayland);
        let control = SniLocalControl {
            runtime_handle: tokio::runtime::Handle::current(),
            kanata: KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone()),
            handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
            status_broadcaster,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle: ShutdownHandle::new(),
            unpause_context: local_sni_unpause_context(Environment::Wayland),
        };
        let control = SniControl::Local(control);

        runtime_environment.set_current(Environment::Kde);
        let _ = take_unpause_request_environment_for_test();
        control.unpause();
        assert_eq!(
            take_unpause_request_environment_for_test(),
            Some(Environment::Wayland),
            "local SNI unpause must use the control's creation environment, not runtime_environment.current()"
        );
    })
    .await;
}

#[tokio::test]
async fn test_sni_runtime_managed_transitions_do_not_leak_watcher_tasks() {
    with_test_timeout(async {
        let _sni_lock = SNI_WATCHER_TEST_LOCK.lock().unwrap();

        async fn assert_sni_watcher_count_eventually(expected: usize, label: &str) {
            for _ in 0..100 {
                if sni_watcher_task_count() == expected {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!(
                "expected {} watcher tasks after {}, got {}",
                expected,
                label,
                sni_watcher_task_count()
            );
        }

        let baseline = sni_watcher_task_count();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));

        let guard = SniGuard::runtime_managed(
            runtime_environment.clone(),
            tokio::runtime::Handle::current(),
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            ShutdownHandle::new(),
            None,
            effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
        );

        assert_sni_watcher_count_eventually(baseline, "initial unknown state").await;

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "first indicator start").await;

        runtime_environment.set_current(Environment::X11);
        assert_sni_watcher_count_eventually(baseline + 3, "same-mode restart").await;

        runtime_environment.set_current(Environment::Unknown);
        assert_sni_watcher_count_eventually(baseline, "indicator stop").await;

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "second indicator start").await;

        drop(guard);
        assert_sni_watcher_count_eventually(baseline, "guard drop").await;
    })
    .await;
}

#[tokio::test]
async fn test_sni_runtime_managed_retries_after_transient_start_failure() {
    with_test_timeout(async {
        let _sni_lock = SNI_WATCHER_TEST_LOCK.lock().unwrap();

        async fn assert_sni_watcher_count_eventually(expected: usize, label: &str) {
            for _ in 0..100 {
                if sni_watcher_task_count() == expected {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!(
                "expected {} watcher tasks after {}, got {}",
                expected,
                label,
                sni_watcher_task_count()
            );
        }

        let baseline = sni_watcher_task_count();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let build_attempts = Arc::new(AtomicUsize::new(0));
        let build_attempts_for_builder = build_attempts.clone();

        let guard = SniGuard::runtime_managed_with_builder(
            runtime_environment.clone(),
            tokio::runtime::Handle::current(),
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            ShutdownHandle::new(),
            None,
            std::time::Duration::from_millis(20),
            move |mode,
                  runtime_handle,
                  kanata,
                  handler,
                  status_broadcaster,
                  pause_broadcaster,
                  restart_handle,
                  shutdown_handle,
                  control_environment,
                  daemon_bus_name| {
                let build_attempts = build_attempts_for_builder.clone();
                async move {
                    let attempt = build_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        None
                    } else {
                        build_sni_control_for_mode(
                            mode,
                            runtime_handle,
                            kanata,
                            handler,
                            status_broadcaster,
                            pause_broadcaster,
                            restart_handle,
                            shutdown_handle,
                            control_environment,
                            daemon_bus_name,
                        )
                        .await
                    }
                }
            },
            effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
        );

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "retry-based indicator recovery").await;
        assert!(
            build_attempts.load(Ordering::SeqCst) >= 2,
            "runtime-managed SNI should retry control construction without an environment change"
        );

        drop(guard);
        assert_sni_watcher_count_eventually(baseline, "guard drop").await;
    })
    .await;
}

#[test]
fn test_map_run_outcome_to_backend_exit() {
    assert_eq!(
        map_run_outcome_to_backend_exit(RunOutcome::Restart),
        BackendExit::Restart
    );
    assert_eq!(
        map_run_outcome_to_backend_exit(RunOutcome::Exit),
        BackendExit::Exit
    );
}
