use super::*;

// === KDE Focus Query Tests ===

struct MockKwinScripting {
    scripts: Arc<Mutex<HashMap<String, i32>>>,
    next_id: Arc<Mutex<i32>>,
    object_server: zbus::ObjectServer,
    is_kde6: bool,
    enforce_kde6_query_api: bool,
    cleanup_behavior: MockKwinCleanupBehavior,
}

#[zbus::interface(name = "org.kde.kwin.Scripting")]
impl MockKwinScripting {
    #[zbus(name = "loadScript")]
    async fn load_script(&self, path: &str) -> i32 {
        let script_id = {
            let mut scripts = self.scripts.lock().unwrap();
            let mut next_id = self.next_id.lock().unwrap();
            let script_id = *next_id;
            *next_id += 1;
            scripts.insert(path.to_string(), script_id);
            script_id
        };
        let obj_path = if self.is_kde6 {
            format!("/Scripting/Script{}", script_id)
        } else {
            format!("/{}", script_id)
        };
        let script = MockKwinScript {
            path: path.to_string(),
            enforce_kde6_query_api: self.enforce_kde6_query_api,
            cleanup_behavior: self.cleanup_behavior,
        };
        self.object_server
            .at(obj_path.as_str(), script)
            .await
            .expect("Failed to register script object");
        script_id
    }

    #[zbus(name = "unloadScript")]
    async fn unload_script(&self, path: &str) {
        let mut scripts = self.scripts.lock().unwrap();
        scripts.remove(path);
    }
}

#[derive(Clone, Copy)]
enum MockKwinCleanupBehavior {
    Normal,
    StopReturnsUnknownObject,
    StopHangs,
}

struct MockKwinScript {
    path: String,
    enforce_kde6_query_api: bool,
    cleanup_behavior: MockKwinCleanupBehavior,
}

#[zbus::interface(name = "org.kde.kwin.Script")]
impl MockKwinScript {
    #[zbus(name = "run")]
    async fn run(&self) -> zbus::fdo::Result<()> {
        let script_contents = std::fs::read_to_string(&self.path).expect("Failed to read script");
        if self.enforce_kde6_query_api {
            if script_contents.contains("workspace.activeClient") {
                return Err(zbus::fdo::Error::Failed(
                    "KDE6 mock rejected KDE5 activeClient query API".to_string(),
                ));
            }
            if !script_contents.contains("workspace.activeWindow") {
                return Err(zbus::fdo::Error::Failed(
                    "KDE6 mock expected activeWindow query API".to_string(),
                ));
            }
        }
        let parts = extract_call_dbus_parts(&script_contents);
        let bus_name = parts.get(0).expect("Missing bus name");
        let object_path = parts.get(1).expect("Missing object path");
        let interface = parts.get(2).expect("Missing interface name");
        let method = parts.get(3).expect("Missing method name");
        let address: zbus::Address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .expect("DBUS_SESSION_BUS_ADDRESS not set")
            .parse()
            .expect("Invalid DBUS_SESSION_BUS_ADDRESS");
        let connection = zbus::connection::Builder::address(address)
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let _ = connection
            .call_method(
                Some(bus_name.as_str()),
                object_path.as_str(),
                Some(interface.as_str()),
                method.as_str(),
                &("kde-app", "KDE Window"),
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    #[zbus(name = "stop")]
    async fn stop(&self) -> zbus::fdo::Result<()> {
        match self.cleanup_behavior {
            MockKwinCleanupBehavior::Normal => Ok(()),
            MockKwinCleanupBehavior::StopReturnsUnknownObject => {
                Err(zbus::fdo::Error::UnknownObject(
                    "No such object path '/Scripting/Script1'".to_string(),
                ))
            }
            MockKwinCleanupBehavior::StopHangs => {
                std::thread::sleep(Duration::from_secs(10));
                Ok(())
            }
        }
    }
}

fn extract_call_dbus_parts(contents: &str) -> Vec<String> {
    let start = contents
        .find("callDBus(")
        .expect("callDBus not found in script");
    let args = &contents[start + "callDBus(".len()..];
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_str = false;
    for ch in args.chars() {
        if ch == '"' {
            if in_str {
                parts.push(current.clone());
                current.clear();
                in_str = false;
            } else {
                in_str = true;
            }
            continue;
        }
        if in_str {
            current.push(ch);
        } else if ch == ')' {
            break;
        }
    }
    parts
}

async fn assert_run_kde_starts_and_stops_with_cleanup_behavior(
    cleanup_behavior: MockKwinCleanupBehavior,
) {
    use zbus::connection::Builder;

    let dbus = DbusSessionGuard::start()
        .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
    let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

    let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());
    let _kde_session_version_env = EnvVarGuard::set("KDE_SESSION_VERSION", "5");

    let scripts = Arc::new(Mutex::new(HashMap::new()));
    let service_connection = Builder::address(address.clone())
        .expect("Failed to create connection builder")
        .name("org.kde.KWin")
        .expect("Failed to set bus name")
        .build()
        .await
        .expect("Failed to build KDE scripting service");
    service_connection
        .object_server()
        .at(
            "/Scripting",
            MockKwinScripting {
                scripts: scripts.clone(),
                next_id: Arc::new(Mutex::new(1)),
                object_server: service_connection.object_server().clone(),
                is_kde6: true,
                enforce_kde6_query_api: true,
                cleanup_behavior,
            },
        )
        .await
        .expect("Failed to register mock scripting interface");

    let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
        .await
        .expect("Failed to create DBus proxy");
    wait_for_async(|| {
        let proxy = dbus_proxy.clone();
        async move {
            proxy
                .name_has_owner("org.kde.KWin".try_into().unwrap())
                .await
                .ok()
                .filter(|&has_owner| has_owner)
        }
    })
    .await
    .expect("Timeout waiting for KDE mock service registration");

    let mock_server = MockKanataServer::start();
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let restart_handle = RestartHandle::new();
    let shutdown_handle = ShutdownHandle::new();
    let rules = vec![Rule {
        class: Some("kde-app".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("terminal".to_string()),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }];
    let kanata = KanataClient::new(
        "127.0.0.1",
        mock_server.port(),
        Some("default".to_string()),
        true,
        status_broadcaster.clone(),
    );
    kanata.connect_with_retry().await;
    drain_kanata_messages(&mock_server, Duration::from_millis(100));

    let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

    let switcher_service_connection = Builder::address(address.clone())
        .expect("Failed to create switcher service builder")
        .build()
        .await
        .expect("Failed to connect switcher service connection");
    let switcher_focus_query_connection = Builder::address(address.clone())
        .expect("Failed to create switcher focus query builder")
        .build()
        .await
        .expect("Failed to connect switcher focus query connection");
    let _dbus_service_guard = register_dbus_service(
        &switcher_service_connection,
        switcher_focus_query_connection,
        Environment::Kde,
        true,
        kanata.clone(),
        handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
        TEST_DAEMON_DBUS_NAME,
    )
    .await
    .expect("Failed to register switcher DBus service");
    let dbus_proxy = zbus::fdo::DBusProxy::new(&switcher_service_connection)
        .await
        .expect("Failed to create switcher DBus proxy");
    wait_for_async(|| {
        let proxy = dbus_proxy.clone();
        async move {
            proxy
                .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                .await
                .ok()
                .filter(|&has_owner| has_owner)
        }
    })
    .await
    .expect("Timeout waiting for switcher DBus service registration");

    let kanata_for_task = kanata.clone();
    let handler_for_task = handler.clone();
    let status_for_task = status_broadcaster.clone();
    let pause_for_task = pause_broadcaster.clone();
    let restart_for_task = restart_handle.clone();
    let shutdown_for_task = shutdown_handle.clone();
    let run_task = tokio::spawn(async move {
        run_kde(
            kanata_for_task,
            handler_for_task,
            status_for_task,
            restart_for_task,
            pause_for_task,
            shutdown_for_task,
            TEST_DAEMON_DBUS_NAME.to_string(),
        )
        .await
    });

    wait_for_kanata_message(
        &mock_server,
        KanataMessage::ChangeLayer {
            new: "terminal".to_string(),
        },
        Duration::from_secs(4),
    );

    shutdown_handle.request();

    let run_result = tokio::time::timeout(Duration::from_secs(8), run_task)
        .await
        .expect("Timed out waiting for KDE backend task");
    let outcome = run_result
        .expect("KDE backend task join failed")
        .expect("KDE backend should start and stop cleanly");
    assert_eq!(outcome, RunOutcome::Exit);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_run_kde_tolerates_kwin_cleanup_unknown_object() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        assert_run_kde_starts_and_stops_with_cleanup_behavior(
            MockKwinCleanupBehavior::StopReturnsUnknownObject,
        )
        .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_run_kde_bounds_kwin_cleanup_latency() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name("org.kde.KWin")
            .expect("Failed to set bus name")
            .build()
            .await
            .expect("Failed to build KDE scripting service");
        service_connection
            .object_server()
            .at(
                "/Scripting",
                MockKwinScripting {
                    scripts: Arc::new(Mutex::new(HashMap::new())),
                    next_id: Arc::new(Mutex::new(2)),
                    object_server: service_connection.object_server().clone(),
                    is_kde6: true,
                    enforce_kde6_query_api: false,
                    cleanup_behavior: MockKwinCleanupBehavior::Normal,
                },
            )
            .await
            .expect("Failed to register mock scripting interface");

        let script_path = kwin_runtime_script_path();
        std::fs::write(&script_path, "function noop() {}\n")
            .expect("failed to write runtime script fixture");
        let script_path_for_assert = script_path.clone();
        let script_obj_path: zbus::zvariant::OwnedObjectPath = "/Scripting/Script1"
            .try_into()
            .expect("valid script object path");
        service_connection
            .object_server()
            .at(
                script_obj_path.as_str(),
                MockKwinScript {
                    path: script_path.clone(),
                    enforce_kde6_query_api: false,
                    cleanup_behavior: MockKwinCleanupBehavior::StopHangs,
                },
            )
            .await
            .expect("Failed to register hanging script object");

        let client_connection = Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");
        let guard = KwinScriptGuard::new(
            client_connection,
            tokio::runtime::Handle::current(),
            script_path,
            script_obj_path,
            "org.kde.kwin.Script",
        );

        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::spawn(async move {
                drop(guard);
            }),
        )
        .await
        .expect("KWin cleanup guard drop should be bounded")
        .expect("KWin cleanup guard drop task should not panic");
        assert!(
            !std::path::Path::new(&script_path_for_assert).exists(),
            "KWin cleanup guard should remove the temporary script file"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_kde_focus_query_on_start_and_unpause() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name("org.kde.KWin")
            .expect("Failed to set bus name")
            .build()
            .await
            .expect("Failed to build scripting service");
        service_connection
            .object_server()
            .at(
                "/Scripting",
                MockKwinScripting {
                    scripts: scripts.clone(),
                    next_id: Arc::new(Mutex::new(1)),
                    object_server: service_connection.object_server().clone(),
                    is_kde6: true,
                    enforce_kde6_query_api: true,
                    cleanup_behavior: MockKwinCleanupBehavior::Normal,
                },
            )
            .await
            .expect("Failed to register mock scripting interface");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner("org.kde.KWin".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for KDE mock service registration");

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("kde-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
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

        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        let client_connection = Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        apply_focus_for_env(
            Environment::Kde,
            Some(&client_connection),
            true,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        .expect("Failed to apply KDE focus on startup");

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "terminal".to_string(),
            },
            Duration::from_secs(2),
        );

        pause_daemon_direct(
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;

        drain_kanata_messages(&mock_server, Duration::from_millis(200));

        unpause_daemon_direct(
            Environment::Kde,
            Some(client_connection.clone()),
            true,
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "terminal".to_string(),
            },
            Duration::from_secs(2),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_unpause_resolves_kde_runtime_mode_without_startup_env() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());
        let _kde_session_version_env = EnvVarGuard::set("KDE_SESSION_VERSION", "5");

        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name("org.kde.KWin")
            .expect("Failed to set bus name")
            .build()
            .await
            .expect("Failed to build KDE scripting service");
        service_connection
            .object_server()
            .at(
                "/Scripting",
                MockKwinScripting {
                    scripts: scripts.clone(),
                    next_id: Arc::new(Mutex::new(1)),
                    object_server: service_connection.object_server().clone(),
                    is_kde6: true,
                    enforce_kde6_query_api: true,
                    cleanup_behavior: MockKwinCleanupBehavior::Normal,
                },
            )
            .await
            .expect("Failed to register mock scripting interface");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner("org.kde.KWin".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for KDE mock service registration");

        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Kde);
        let rules = vec![Rule {
            class: Some("kde-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let daemon_connection = Builder::address(address.clone())
            .expect("Failed to create daemon connection builder")
            .build()
            .await
            .expect("Failed to connect daemon connection");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query connection builder")
            .build()
            .await
            .expect("Failed to connect focus query connection");
        let _dbus_service_guard = register_dbus_service_with_runtime_environment(
            &daemon_connection,
            focus_query_connection,
            Environment::Unknown,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster.clone(),
            Some(runtime_environment),
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register DBus service with runtime environment");

        let client = Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let pause_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Pause).await;
        assert!(
            pause_result.is_ok(),
            "Pause control command failed: {:?}",
            pause_result.err()
        );
        drain_kanata_messages(&mock_server, Duration::from_millis(200));

        let unpause_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Unpause).await;
        assert!(
            unpause_result.is_ok(),
            "Unpause control command failed: {:?}",
            unpause_result.err()
        );

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "terminal".to_string(),
            },
            Duration::from_secs(2),
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_run_kde_resolves_runtime_mode_without_startup_env() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());
        let _kde_session_version_env = EnvVarGuard::set("KDE_SESSION_VERSION", "5");

        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name("org.kde.KWin")
            .expect("Failed to set bus name")
            .build()
            .await
            .expect("Failed to build KDE scripting service");
        service_connection
            .object_server()
            .at(
                "/Scripting",
                MockKwinScripting {
                    scripts: scripts.clone(),
                    next_id: Arc::new(Mutex::new(1)),
                    object_server: service_connection.object_server().clone(),
                    is_kde6: true,
                    enforce_kde6_query_api: true,
                    cleanup_behavior: MockKwinCleanupBehavior::Normal,
                },
            )
            .await
            .expect("Failed to register mock scripting interface");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner("org.kde.KWin".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for KDE mock service registration");

        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let rules = vec![Rule {
            class: Some("kde-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        let switcher_service_connection = Builder::address(address.clone())
            .expect("Failed to create switcher service builder")
            .build()
            .await
            .expect("Failed to connect switcher service connection");
        let switcher_focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create switcher focus query builder")
            .build()
            .await
            .expect("Failed to connect switcher focus query connection");
        let _dbus_service_guard = register_dbus_service(
            &switcher_service_connection,
            switcher_focus_query_connection,
            Environment::Kde,
            true,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            restart_handle.clone(),
            pause_broadcaster.clone(),
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register switcher DBus service");
        let dbus_proxy = zbus::fdo::DBusProxy::new(&switcher_service_connection)
            .await
            .expect("Failed to create switcher DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for switcher DBus service registration");

        let kanata_for_task = kanata.clone();
        let handler_for_task = handler.clone();
        let status_for_task = status_broadcaster.clone();
        let pause_for_task = pause_broadcaster.clone();
        let restart_for_task = restart_handle.clone();
        let shutdown_for_task = shutdown_handle.clone();
        let run_task = tokio::spawn(async move {
            run_kde(
                kanata_for_task,
                handler_for_task,
                status_for_task,
                restart_for_task,
                pause_for_task,
                shutdown_for_task,
                TEST_DAEMON_DBUS_NAME.to_string(),
            )
            .await
        });

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "terminal".to_string(),
            },
            Duration::from_secs(2),
        );

        shutdown_handle.request();

        let run_result = tokio::time::timeout(Duration::from_secs(2), run_task)
            .await
            .expect("Timed out waiting for KDE backend task");
        let outcome = run_result
            .expect("KDE backend task join failed")
            .expect("KDE backend should start and stop cleanly");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_run_kde_waits_for_scripting_interface_before_runtime_probe() {
    let _dbus_env_lock = DBUS_ENV_LOCK.lock().unwrap();
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let _dbus_address_env = EnvVarGuard::set("DBUS_SESSION_BUS_ADDRESS", dbus.address());
        let _kde_session_version_env = EnvVarGuard::set("KDE_SESSION_VERSION", "5");

        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let scripting_next_id = Arc::new(Mutex::new(1));
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name("org.kde.KWin")
            .expect("Failed to set bus name")
            .build()
            .await
            .expect("Failed to build KDE scripting service");

        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner("org.kde.KWin".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for KDE mock service registration");

        let delayed_object_server = service_connection.object_server().clone();
        let delayed_scripts = scripts.clone();
        let delayed_next_id = scripting_next_id.clone();
        let delayed_registration = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            delayed_object_server
                .at(
                    "/Scripting",
                    MockKwinScripting {
                        scripts: delayed_scripts,
                        next_id: delayed_next_id,
                        object_server: delayed_object_server.clone(),
                        is_kde6: true,
                        enforce_kde6_query_api: true,
                        cleanup_behavior: MockKwinCleanupBehavior::Normal,
                    },
                )
                .await
                .expect("Failed to register delayed mock scripting interface");
        });

        let mock_server = MockKanataServer::start();
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let rules = vec![Rule {
            class: Some("kde-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("terminal".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;
        drain_kanata_messages(&mock_server, Duration::from_millis(100));

        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        let switcher_service_connection = Builder::address(address.clone())
            .expect("Failed to create switcher service builder")
            .build()
            .await
            .expect("Failed to connect switcher service connection");
        let switcher_focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create switcher focus query builder")
            .build()
            .await
            .expect("Failed to connect switcher focus query connection");
        let _dbus_service_guard = register_dbus_service(
            &switcher_service_connection,
            switcher_focus_query_connection,
            Environment::Kde,
            true,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            restart_handle.clone(),
            pause_broadcaster.clone(),
            TEST_DAEMON_DBUS_NAME,
        )
        .await
        .expect("Failed to register switcher DBus service");
        let switcher_proxy = zbus::fdo::DBusProxy::new(&switcher_service_connection)
            .await
            .expect("Failed to create switcher DBus proxy");
        wait_for_async(|| {
            let proxy = switcher_proxy.clone();
            async move {
                proxy
                    .name_has_owner(TEST_DAEMON_DBUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for switcher DBus service registration");

        let kanata_for_task = kanata.clone();
        let handler_for_task = handler.clone();
        let status_for_task = status_broadcaster.clone();
        let pause_for_task = pause_broadcaster.clone();
        let restart_for_task = restart_handle.clone();
        let shutdown_for_task = shutdown_handle.clone();
        let run_task = tokio::spawn(async move {
            run_kde(
                kanata_for_task,
                handler_for_task,
                status_for_task,
                restart_for_task,
                pause_for_task,
                shutdown_for_task,
                TEST_DAEMON_DBUS_NAME.to_string(),
            )
            .await
        });

        wait_for_kanata_message(
            &mock_server,
            KanataMessage::ChangeLayer {
                new: "terminal".to_string(),
            },
            Duration::from_secs(3),
        );

        delayed_registration
            .await
            .expect("Delayed scripting registration task failed");

        shutdown_handle.request();

        let run_result = tokio::time::timeout(Duration::from_secs(2), run_task)
            .await
            .expect("Timed out waiting for KDE backend task");
        let outcome = run_result
            .expect("KDE backend task join failed")
            .expect("KDE backend should start and stop cleanly after scripting appears");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}
