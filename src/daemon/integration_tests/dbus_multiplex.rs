use super::*;

// === DBus multi-instance integration tests ===

const TEST_DAEMON_DBUS_NAME_A: &str = "com.github.kanata.Switcher.instances.a";
const TEST_DAEMON_DBUS_NAME_B: &str = "com.github.kanata.Switcher.instances.b";

async fn register_test_daemon_with_name(
    address: &zbus::Address,
    bus_name: &str,
    pause_broadcaster: PauseBroadcaster,
    handler: Arc<Mutex<FocusHandler>>,
    kanata: KanataClient,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
) -> (Connection, DbusServiceRegistration) {
    use zbus::connection::Builder;
    let service_connection = Builder::address(address.clone())
        .expect("Failed to build service connection")
        .build()
        .await
        .expect("Failed to connect service to bus");
    let focus_query_connection = Builder::address(address.clone())
        .expect("Failed to build focus query connection")
        .build()
        .await
        .expect("Failed to connect focus query to bus");
    let registration = register_dbus_service(
        &service_connection,
        focus_query_connection,
        Environment::Unknown,
        false,
        kanata,
        handler,
        status_broadcaster,
        restart_handle,
        pause_broadcaster,
        bus_name,
    )
    .await
    .expect("Failed to register named daemon service");
    (service_connection, registration)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_two_daemons_register_independent_names() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let server_a = MockKanataServer::start();
        let server_b = MockKanataServer::start();
        let status_a = StatusBroadcaster::new();
        let status_b = StatusBroadcaster::new();
        let kanata_a = KanataClient::new(
            "127.0.0.1",
            server_a.port(),
            Some("default".to_string()),
            true,
            status_a.clone(),
        );
        let kanata_b = KanataClient::new(
            "127.0.0.1",
            server_b.port(),
            Some("default".to_string()),
            true,
            status_b.clone(),
        );
        kanata_a.connect_with_retry().await;
        kanata_b.connect_with_retry().await;
        drain_kanata_messages(&server_a, Duration::from_millis(100));
        drain_kanata_messages(&server_b, Duration::from_millis(100));

        let handler_a = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let handler_b = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause_a = PauseBroadcaster::new();
        let pause_b = PauseBroadcaster::new();
        let (_conn_a, _reg_a) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause_a.clone(),
            handler_a,
            kanata_a,
            status_a,
            RestartHandle::new(),
        )
        .await;
        let (_conn_b, _reg_b) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_B,
            pause_b.clone(),
            handler_b,
            kanata_b,
            status_b,
            RestartHandle::new(),
        )
        .await;

        let client = zbus::connection::Builder::address(address)
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");

        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A registration");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_B.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon B registration");

        // Bare base name and root not owned.
        let bare_owned = dbus_proxy
            .name_has_owner("com.github.kanata.Switcher.instances".try_into().unwrap())
            .await
            .unwrap_or(true);
        assert!(
            !bare_owned,
            "Bare prefix com.github.kanata.Switcher.instances should not be owned"
        );
        let root_owned = dbus_proxy
            .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
            .await
            .unwrap_or(true);
        assert!(
            !root_owned,
            "Root com.github.kanata.Switcher must not be owned in instances mode"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_targets_specific_daemon_when_suffix_given() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let server_a = MockKanataServer::start();
        let server_b = MockKanataServer::start();
        let status_a = StatusBroadcaster::new();
        let status_b = StatusBroadcaster::new();
        let kanata_a = KanataClient::new(
            "127.0.0.1",
            server_a.port(),
            Some("default".to_string()),
            true,
            status_a.clone(),
        );
        let kanata_b = KanataClient::new(
            "127.0.0.1",
            server_b.port(),
            Some("default".to_string()),
            true,
            status_b.clone(),
        );
        kanata_a.connect_with_retry().await;
        kanata_b.connect_with_retry().await;
        drain_kanata_messages(&server_a, Duration::from_millis(100));
        drain_kanata_messages(&server_b, Duration::from_millis(100));

        let handler_a = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let handler_b = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause_a = PauseBroadcaster::new();
        let pause_b = PauseBroadcaster::new();
        let mut pause_a_rx = pause_a.subscribe();
        let mut pause_b_rx = pause_b.subscribe();
        let (_conn_a, _reg_a) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause_a.clone(),
            handler_a,
            kanata_a,
            status_a,
            RestartHandle::new(),
        )
        .await;
        let (_conn_b, _reg_b) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_B,
            pause_b.clone(),
            handler_b,
            kanata_b,
            status_b,
            RestartHandle::new(),
        )
        .await;

        let client = zbus::connection::Builder::address(address)
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_B.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon B");

        send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME_A, ControlCommand::Pause)
            .await
            .expect("Pause unicast call failed");

        // Wait for A pause broadcast
        tokio::time::timeout(Duration::from_secs(2), pause_a_rx.changed())
            .await
            .expect("Daemon A did not flip paused state")
            .expect("Daemon A pause channel closed");
        assert!(*pause_a_rx.borrow(), "Daemon A should be paused");

        // Daemon B's broadcaster should not have changed; give it a small grace window
        let observed_b =
            tokio::time::timeout(Duration::from_millis(200), pause_b_rx.changed()).await;
        assert!(
            observed_b.is_err(),
            "Daemon B unexpectedly observed a pause change after unicast targeted A"
        );
        assert!(
            !*pause_b_rx.borrow(),
            "Daemon B should remain unpaused after unicast to A"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_broadcasts_when_suffix_absent() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let server_a = MockKanataServer::start();
        let server_b = MockKanataServer::start();
        let status_a = StatusBroadcaster::new();
        let status_b = StatusBroadcaster::new();
        let kanata_a = KanataClient::new(
            "127.0.0.1",
            server_a.port(),
            Some("default".to_string()),
            true,
            status_a.clone(),
        );
        let kanata_b = KanataClient::new(
            "127.0.0.1",
            server_b.port(),
            Some("default".to_string()),
            true,
            status_b.clone(),
        );
        kanata_a.connect_with_retry().await;
        kanata_b.connect_with_retry().await;
        drain_kanata_messages(&server_a, Duration::from_millis(100));
        drain_kanata_messages(&server_b, Duration::from_millis(100));

        let handler_a = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let handler_b = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause_a = PauseBroadcaster::new();
        let pause_b = PauseBroadcaster::new();
        let mut pause_a_rx = pause_a.subscribe();
        let mut pause_b_rx = pause_b.subscribe();
        let (_conn_a, _reg_a) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause_a.clone(),
            handler_a,
            kanata_a,
            status_a,
            RestartHandle::new(),
        )
        .await;
        let (_conn_b, _reg_b) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_B,
            pause_b.clone(),
            handler_b,
            kanata_b,
            status_b,
            RestartHandle::new(),
        )
        .await;

        let client = zbus::connection::Builder::address(address)
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_B.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon B");

        let report =
            send_control_command_broadcast(&client, ControlCommand::Pause)
                .await
                .expect("Broadcast Pause failed");
        let names: Vec<&str> = report
            .results
            .iter()
            .map(|entry| entry.bus_name.as_str())
            .collect();
        assert!(
            names.contains(&TEST_DAEMON_DBUS_NAME_A),
            "broadcast missed daemon A: got {:?}",
            names
        );
        assert!(
            names.contains(&TEST_DAEMON_DBUS_NAME_B),
            "broadcast missed daemon B: got {:?}",
            names
        );
        for entry in &report.results {
            assert!(
                entry.outcome.is_ok(),
                "broadcast to {} reported error: {:?}",
                entry.bus_name,
                entry.outcome
            );
        }

        tokio::time::timeout(Duration::from_secs(2), pause_a_rx.changed())
            .await
            .expect("Daemon A did not flip paused")
            .expect("Daemon A pause channel closed");
        tokio::time::timeout(Duration::from_secs(2), pause_b_rx.changed())
            .await
            .expect("Daemon B did not flip paused")
            .expect("Daemon B pause channel closed");
        assert!(*pause_a_rx.borrow(), "Daemon A should be paused");
        assert!(*pause_b_rx.borrow(), "Daemon B should be paused");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_broadcast_with_no_daemons_errors() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let client = zbus::connection::Builder::address(address)
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");

        let outcome = send_control_command_broadcast(&client, ControlCommand::Pause).await;
        let error = outcome.expect_err("Expected broadcast to fail with no daemons");
        let message = error.to_string();
        assert!(
            message.contains("No daemons running"),
            "expected 'No daemons running' in error message, got: {}",
            message
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_enumerate_daemon_names_ignores_unrelated_namespaces() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let server = MockKanataServer::start();
        let status = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&server, Duration::from_millis(100));

        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause = PauseBroadcaster::new();
        let (_conn, _reg) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause,
            handler,
            kanata,
            status,
            RestartHandle::new(),
        )
        .await;

        // Register an unrelated name that shares the project root prefix but is
        // outside the `instances.*` subtree (it lives in `extensions.*`-shaped path
        // but as a bus name). The filter must drop it.
        let imposter_connection = Builder::address(address.clone())
            .expect("Failed to build imposter connection")
            .name("com.github.kanata.Switcher.extensions.GNOME")
            .expect("Failed to set imposter bus name")
            .build()
            .await
            .expect("Failed to register imposter name");

        let client = Builder::address(address.clone())
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A");

        let names = enumerate_daemon_names(&client)
            .await
            .expect("enumerate_daemon_names failed");
        assert_eq!(
            names,
            vec![TEST_DAEMON_DBUS_NAME_A.to_string()],
            "Daemon enumeration must not include the extensions.* imposter; got {:?}",
            names
        );

        drop(imposter_connection);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_targeted_unknown_suffix_errors() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let client = zbus::connection::Builder::address(address)
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");

        let bus_name = "com.github.kanata.Switcher.instances.does_not_exist";
        let outcome =
            send_control_command_with_connection(&client, bus_name, ControlCommand::Pause).await;
        let error = outcome.expect_err("Expected unicast to unknown suffix to fail");
        let message = error.to_string();
        assert!(
            message.contains("does_not_exist") || message.contains("ServiceUnknown") || message.contains("NameHasNoOwner"),
            "expected target name in error or DBus 'unknown service' marker, got: {}",
            message
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_gnome_daemon_subscribes_to_focus_signal() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Mock GNOME Shell stand-in: owns org.gnome.Shell, exports the renamed
        // extensions.GNOME object at the renamed path, and can emit FocusChanged.
        let mock_extension_connection = Builder::address(address.clone())
            .expect("Failed to build extension connection builder")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to claim org.gnome.Shell")
            .serve_at(
                GNOME_FOCUS_OBJECT_PATH,
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount extensions.GNOME interface")
            .build()
            .await
            .expect("Failed to build mock extension connection");

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let signal_connection = Builder::address(address.clone())
            .expect("Failed to build signal connection")
            .build()
            .await
            .expect("Failed to connect signal listener");
        let _subscription = subscribe_to_gnome_focus_signal(
            &signal_connection,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
        )
        .await
        .expect("Failed to subscribe to FocusChanged signal");

        // Give zbus a moment to register the match rule on the bus.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let signal_emitter =
            zbus::object_server::SignalEmitter::new(&mock_extension_connection, GNOME_FOCUS_OBJECT_PATH)
                .expect("Failed to create signal emitter");
        FocusService::focus_changed(&signal_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged signal");

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "browser".to_string(),
            },
            Duration::from_secs(3),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_gnome_daemon_multiple_instances_receive_focus_signal() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let mock_extension_connection = Builder::address(address.clone())
            .expect("Failed to build extension connection builder")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to claim org.gnome.Shell")
            .serve_at(
                GNOME_FOCUS_OBJECT_PATH,
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount extensions.GNOME interface")
            .build()
            .await
            .expect("Failed to build mock extension connection");

        let server_a = MockKanataServer::start();
        let server_b = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let status_a = StatusBroadcaster::new();
        let status_b = StatusBroadcaster::new();
        let kanata_a = KanataClient::new(
            "127.0.0.1",
            server_a.port(),
            Some("default".to_string()),
            true,
            status_a.clone(),
        );
        let kanata_b = KanataClient::new(
            "127.0.0.1",
            server_b.port(),
            Some("default".to_string()),
            true,
            status_b.clone(),
        );
        kanata_a.connect_with_retry().await;
        kanata_b.connect_with_retry().await;
        drain_kanata_messages(&server_a, Duration::from_millis(100));
        drain_kanata_messages(&server_b, Duration::from_millis(100));

        let handler_a = Arc::new(Mutex::new(FocusHandler::new(rules.clone(), None, true)));
        let handler_b = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_a = PauseBroadcaster::new();
        let pause_b = PauseBroadcaster::new();

        let signal_connection_a = Builder::address(address.clone())
            .expect("Failed to build signal connection A")
            .build()
            .await
            .expect("Failed to connect A");
        let signal_connection_b = Builder::address(address.clone())
            .expect("Failed to build signal connection B")
            .build()
            .await
            .expect("Failed to connect B");
        let _sub_a = subscribe_to_gnome_focus_signal(
            &signal_connection_a,
            kanata_a.clone(),
            handler_a,
            status_a,
            pause_a,
        )
        .await
        .expect("Failed to subscribe A");
        let _sub_b = subscribe_to_gnome_focus_signal(
            &signal_connection_b,
            kanata_b.clone(),
            handler_b,
            status_b,
            pause_b,
        )
        .await
        .expect("Failed to subscribe B");

        tokio::time::sleep(Duration::from_millis(150)).await;

        let signal_emitter =
            zbus::object_server::SignalEmitter::new(&mock_extension_connection, GNOME_FOCUS_OBJECT_PATH)
                .expect("Failed to create signal emitter");
        FocusService::focus_changed(&signal_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged signal");

        wait_for_kanata_message(
            &server_a,
            KanataMessage::ChangeLayer {
                new: "browser".to_string(),
            },
            Duration::from_secs(3),
        );
        wait_for_kanata_message(
            &server_b,
            KanataMessage::ChangeLayer {
                new: "browser".to_string(),
            },
            Duration::from_secs(3),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_persistent_dbus_service_reconnect_uses_effective_name() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        let effective_name = "com.github.kanata.Switcher.instances.kinesis".to_string();
        let connector_address = address.clone();
        let _dbus_guard = start_persistent_dbus_service_with_connector(
            move || {
                let address = connector_address.clone();
                async move {
                    Builder::address(address)
                        .expect("Failed to create connection builder")
                        .build()
                        .await
                        .map_err(|error| -> DynError { Box::new(error) })
                }
            },
            kanata,
            handler,
            status_broadcaster,
            restart_handle.clone(),
            pause_broadcaster,
            runtime_environment,
            shutdown_handle.clone(),
            effective_name.clone(),
        );

        let client = Builder::address(address.clone())
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            let name = effective_name.clone();
            async move {
                proxy
                    .name_has_owner(name.as_str().try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for persistent DBus service registration");

        let restart_result =
            send_control_command_with_connection(&client, &effective_name, ControlCommand::Restart)
                .await;
        assert!(
            restart_result.is_ok(),
            "Restart control command failed: {:?}",
            restart_result.err()
        );

        let mut restart_receiver = restart_handle.subscribe();
        if !*restart_receiver.borrow() {
            tokio::time::timeout(Duration::from_secs(2), restart_receiver.changed())
                .await
                .expect("Timeout waiting for restart broadcast")
                .expect("Restart broadcast stream closed");
        }
        assert!(
            *restart_receiver.borrow(),
            "Expected restart handle to be requested via persistent DBus service"
        );

        shutdown_handle.request();
    })
    .await;
}

/// Stand-in for a daemon whose control methods take forever. Used to exercise
/// `send_control_command_broadcast`'s per-call `tokio::time::timeout` cap.
struct HangingControlService;

#[zbus::interface(name = "com.github.kanata.Switcher")]
impl HangingControlService {
    async fn pause(&self) {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    async fn unpause(&self) {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    async fn restart(&self) {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_broadcast_times_out_hanging_daemon_per_call() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Daemon A: full registration that responds to Pause quickly.
        let server = MockKanataServer::start();
        let status = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&server, Duration::from_millis(100));
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause_a = PauseBroadcaster::new();
        let mut pause_a_rx = pause_a.subscribe();
        let (_conn_a, _reg_a) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause_a.clone(),
            handler,
            kanata,
            status,
            RestartHandle::new(),
        )
        .await;

        // Daemon B: claims the well-known name and serves the control interface
        // at the daemon object path, but every method handler sleeps for 60s.
        // The broadcast's per-call timeout must cut this off.
        let hanging_connection = Builder::address(address.clone())
            .expect("Failed to build hanging daemon connection")
            .name(TEST_DAEMON_DBUS_NAME_B)
            .expect("Failed to claim hanging daemon name")
            .serve_at("/com/github/kanata/Switcher", HangingControlService)
            .expect("Failed to mount hanging service")
            .build()
            .await
            .expect("Failed to register hanging daemon");

        let client = Builder::address(address.clone())
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_B.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon B (hanging)");

        let started = std::time::Instant::now();
        let report = send_control_command_broadcast(&client, ControlCommand::Pause)
            .await
            .expect("Broadcast should aggregate, not propagate the timeout");
        let elapsed = started.elapsed();
        // Total time should be roughly bounded by `BROADCAST_PER_CALL_TIMEOUT`
        // (2s) plus daemon A's near-zero call time. Generous slack avoids
        // flake while still proving the cap is enforced (a missing timeout
        // would run for 60s and the outer `with_test_timeout(5s)` would fire).
        assert!(
            elapsed < Duration::from_millis(3500),
            "broadcast must respect per-call timeout; took {:?}",
            elapsed
        );

        let mut by_name: std::collections::HashMap<&str, &BroadcastEntryReport> =
            std::collections::HashMap::new();
        for entry in &report.results {
            by_name.insert(entry.bus_name.as_str(), entry);
        }
        let a_entry = by_name
            .get(TEST_DAEMON_DBUS_NAME_A)
            .expect("Report must include daemon A");
        let b_entry = by_name
            .get(TEST_DAEMON_DBUS_NAME_B)
            .expect("Report must include daemon B");
        assert!(
            a_entry.outcome.is_ok(),
            "Daemon A should succeed, got: {:?}",
            a_entry.outcome
        );
        let b_error = b_entry
            .outcome
            .as_ref()
            .err()
            .expect("Daemon B (hanging) must time out");
        assert!(
            b_error.to_string().contains("timed out"),
            "Daemon B error must mention timeout, got: {}",
            b_error
        );

        tokio::time::timeout(Duration::from_secs(2), pause_a_rx.changed())
            .await
            .expect("Daemon A did not flip paused")
            .expect("Daemon A pause channel closed");
        assert!(*pause_a_rx.borrow(), "Daemon A should be paused");

        drop(hanging_connection);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_broadcast_dispatches_in_parallel() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Two daemons that both hang on Pause for 60s. With sequential dispatch
        // the broadcast would take roughly `2 * BROADCAST_PER_CALL_TIMEOUT`
        // (~4s); with parallel dispatch it should complete in roughly one
        // per-call timeout (~2s). The assertion below proves the parallel
        // path is in effect.
        let hanging_a = Builder::address(address.clone())
            .expect("Failed to build hanging A connection")
            .name(TEST_DAEMON_DBUS_NAME_A)
            .expect("Failed to claim hanging A name")
            .serve_at("/com/github/kanata/Switcher", HangingControlService)
            .expect("Failed to mount hanging A service")
            .build()
            .await
            .expect("Failed to register hanging A daemon");
        let hanging_b = Builder::address(address.clone())
            .expect("Failed to build hanging B connection")
            .name(TEST_DAEMON_DBUS_NAME_B)
            .expect("Failed to claim hanging B name")
            .serve_at("/com/github/kanata/Switcher", HangingControlService)
            .expect("Failed to mount hanging B service")
            .build()
            .await
            .expect("Failed to register hanging B daemon");

        let client = Builder::address(address.clone())
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        for name in [TEST_DAEMON_DBUS_NAME_A, TEST_DAEMON_DBUS_NAME_B] {
            wait_for_async(|| {
                let proxy = dbus_proxy.clone();
                async move {
                    proxy
                        .name_has_owner(name.try_into().unwrap())
                        .await
                        .ok()
                        .filter(|&owned| owned)
                }
            })
            .await
            .unwrap_or_else(|_| panic!("Timeout waiting for {}", name));
        }

        let started = std::time::Instant::now();
        let report = send_control_command_broadcast(&client, ControlCommand::Pause)
            .await
            .expect("Broadcast must not fail just because every call timed out (report still aggregated)");
        let elapsed = started.elapsed();
        // Sequential would be ~4s. Parallel should be ~2s. Allow ~1s slack
        // for scheduling/zbus overhead but stay well below the sequential
        // total to prove parallelism.
        assert!(
            elapsed < Duration::from_millis(3000),
            "broadcast must dispatch in parallel; took {:?} (would be ~4s sequentially)",
            elapsed
        );
        assert_eq!(report.results.len(), 2);
        for entry in &report.results {
            let error = entry
                .outcome
                .as_ref()
                .err()
                .unwrap_or_else(|| panic!("Hanging daemon {} should time out", entry.bus_name));
            assert!(
                error.to_string().contains("timed out"),
                "Hanging daemon {} error must mention timeout, got: {}",
                entry.bus_name,
                error
            );
        }

        drop(hanging_a);
        drop(hanging_b);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_focus_changed_signal_ignores_unrelated_sender() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Correct GNOME Shell stand-in (owns `org.gnome.Shell`).
        let mock_gshell_connection = Builder::address(address.clone())
            .expect("Failed to build GShell mock connection")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to claim org.gnome.Shell")
            .serve_at(
                GNOME_FOCUS_OBJECT_PATH,
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount correct GShell mock")
            .build()
            .await
            .expect("Failed to build GShell mock connection");

        // Impostor: owns a *different* well-known name but serves the same
        // interface + path. Signals emitted from this connection have a
        // different sender bus name and must be rejected by the daemon's
        // `sender=org.gnome.Shell` MatchRule filter.
        let impostor_connection = Builder::address(address.clone())
            .expect("Failed to build impostor connection")
            .name("org.example.NotGnomeShell")
            .expect("Failed to claim impostor name")
            .serve_at(
                GNOME_FOCUS_OBJECT_PATH,
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount impostor")
            .build()
            .await
            .expect("Failed to build impostor connection");

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let signal_connection = Builder::address(address.clone())
            .expect("Failed to build signal connection")
            .build()
            .await
            .expect("Failed to connect signal listener");
        let _subscription = subscribe_to_gnome_focus_signal(
            &signal_connection,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
        )
        .await
        .expect("Failed to subscribe to FocusChanged signal");
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Emit FocusChanged from the impostor — must be ignored by the daemon.
        let impostor_emitter =
            zbus::object_server::SignalEmitter::new(&impostor_connection, GNOME_FOCUS_OBJECT_PATH)
                .expect("Failed to create impostor signal emitter");
        FocusService::focus_changed(&impostor_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged from impostor");
        let unexpected = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(
            unexpected.is_none(),
            "Daemon must ignore FocusChanged from non-org.gnome.Shell sender, but got: {:?}",
            unexpected
        );

        // Sanity: emit from the legitimate `org.gnome.Shell` owner and verify
        // the daemon reacts. Without this, a daemon that ignored *every*
        // signal would also pass the negative assertion above.
        let legit_emitter =
            zbus::object_server::SignalEmitter::new(&mock_gshell_connection, GNOME_FOCUS_OBJECT_PATH)
                .expect("Failed to create legit signal emitter");
        FocusService::focus_changed(&legit_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged from legit sender");
        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "browser".to_string(),
            },
            Duration::from_secs(3),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_broadcast_partial_failure_reports_per_daemon() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Daemon A: full registration that responds to Pause.
        let server = MockKanataServer::start();
        let status = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&server, Duration::from_millis(100));
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let pause_a = PauseBroadcaster::new();
        let mut pause_a_rx = pause_a.subscribe();
        let (_conn_a, _reg_a) = register_test_daemon_with_name(
            &address,
            TEST_DAEMON_DBUS_NAME_A,
            pause_a.clone(),
            handler,
            kanata,
            status,
            RestartHandle::new(),
        )
        .await;

        // Daemon B: own the well-known name but expose no object at the daemon
        // path. Method calls return a DBus error → per-entry Err in the report.
        let broken_connection = Builder::address(address.clone())
            .expect("Failed to build broken daemon connection")
            .name(TEST_DAEMON_DBUS_NAME_B)
            .expect("Failed to claim broken daemon name")
            .build()
            .await
            .expect("Failed to register broken daemon name");

        let client = Builder::address(address.clone())
            .expect("Failed to build client")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create dbus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_A.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon A");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME_B.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&owned| owned)
            }
        })
        .await
        .expect("Timeout waiting for daemon B (broken)");

        let started = std::time::Instant::now();
        let report = send_control_command_broadcast(&client, ControlCommand::Pause)
            .await
            .expect("Broadcast should aggregate per-daemon outcomes, not fail when at least one daemon responds");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "broadcast should complete within bounded time, took {:?}",
            started.elapsed()
        );

        let mut by_name: std::collections::HashMap<&str, &BroadcastEntryReport> =
            std::collections::HashMap::new();
        for entry in &report.results {
            by_name.insert(entry.bus_name.as_str(), entry);
        }
        let a_entry = by_name
            .get(TEST_DAEMON_DBUS_NAME_A)
            .expect("Report must include daemon A");
        let b_entry = by_name
            .get(TEST_DAEMON_DBUS_NAME_B)
            .expect("Report must include daemon B");
        assert!(
            a_entry.outcome.is_ok(),
            "Daemon A should succeed, got: {:?}",
            a_entry.outcome
        );
        assert!(
            b_entry.outcome.is_err(),
            "Daemon B (no object server) should error, got: {:?}",
            b_entry.outcome
        );

        // Daemon A should have actually paused.
        tokio::time::timeout(Duration::from_secs(2), pause_a_rx.changed())
            .await
            .expect("Daemon A did not flip paused")
            .expect("Daemon A pause channel closed");
        assert!(*pause_a_rx.borrow(), "Daemon A should be paused");

        drop(broken_connection);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_focus_changed_signal_filters_by_path() {
    with_test_timeout(async {
        use zbus::connection::Builder;
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Mock extension owns org.gnome.Shell and serves the FocusService at the
        // correct path; we'll emit signals from a *different* path with the
        // same interface+member, and verify the daemon ignores them.
        let mock_extension_connection = Builder::address(address.clone())
            .expect("Failed to build extension connection builder")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to claim org.gnome.Shell")
            .serve_at(
                GNOME_FOCUS_OBJECT_PATH,
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount extensions.GNOME at correct path")
            .serve_at(
                "/com/github/kanata/Switcher/extensions/UNKNOWN",
                FocusService {
                    call_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    class: "firefox".to_string(),
                    title: "".to_string(),
                },
            )
            .expect("Failed to mount extensions.GNOME at wrong path for the test")
            .build()
            .await
            .expect("Failed to build mock extension connection");

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let signal_connection = Builder::address(address.clone())
            .expect("Failed to build signal connection")
            .build()
            .await
            .expect("Failed to connect signal listener");
        let _subscription = subscribe_to_gnome_focus_signal(
            &signal_connection,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
        )
        .await
        .expect("Failed to subscribe to FocusChanged signal");
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Emit a FocusChanged signal on the WRONG path. Daemon's MatchRule
        // pins `path=/com/github/kanata/Switcher/extensions/GNOME`, so this
        // signal must be ignored.
        let wrong_path_emitter = zbus::object_server::SignalEmitter::new(
            &mock_extension_connection,
            "/com/github/kanata/Switcher/extensions/UNKNOWN",
        )
        .expect("Failed to create wrong-path signal emitter");
        FocusService::focus_changed(&wrong_path_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged from wrong path");

        // Give zbus a chance to deliver — daemon must NOT act.
        let unexpected = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(
            unexpected.is_none(),
            "Daemon must ignore FocusChanged on wrong path, but got: {:?}",
            unexpected
        );

        // Sanity: emit on the CORRECT path and verify the daemon does react.
        // This guards against a false-positive (the daemon ignoring everything
        // would also make the negative assertion pass).
        let correct_emitter =
            zbus::object_server::SignalEmitter::new(&mock_extension_connection, GNOME_FOCUS_OBJECT_PATH)
                .expect("Failed to create correct-path signal emitter");
        FocusService::focus_changed(&correct_emitter, "firefox", "")
            .await
            .expect("Failed to emit FocusChanged from correct path");
        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "browser".to_string(),
            },
            Duration::from_secs(3),
        );
    })
    .await;
}

#[test]
fn test_kde_focus_push_script_targets_per_instance_name() {
    let script = build_kde_focus_push_script(
        "com.github.kanata.Switcher.instances.kinesis",
        "windowActivated",
        "activeWindow",
    );
    assert!(
        script.contains("\"com.github.kanata.Switcher.instances.kinesis\""),
        "expected per-instance bus name in script body, got: {}",
        script
    );
    // The KWin script's `callDBus` puts the bus name on the first line and the
    // interface (constant `com.github.kanata.Switcher`) on the third. We assert
    // that the *bus-name* argument — the first quoted token inside `callDBus(`
    // — is the per-instance name.
    let body_after_call = script
        .split_once("callDBus(")
        .expect("script must contain callDBus(")
        .1;
    let first_arg = body_after_call
        .lines()
        .map(|line| line.trim())
        .find(|line| line.starts_with('"'))
        .expect("first callDBus arg should be a quoted string");
    assert!(
        first_arg.starts_with("\"com.github.kanata.Switcher.instances.kinesis\""),
        "first callDBus arg should be the per-instance bus name, got: {}",
        first_arg
    );
}
