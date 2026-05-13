use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_persistent_dbus_service_handles_restart_in_idle_runtime() {
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
            TEST_DAEMON_DBUS_NAME.to_string(),
        );

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
        .expect("Timeout waiting for persistent DBus service registration");

        let restart_result =
            send_control_command_with_connection(&client, TEST_DAEMON_DBUS_NAME, ControlCommand::Restart).await;
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
