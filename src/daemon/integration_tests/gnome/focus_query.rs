use super::*;

// === GNOME Focus Query Tests ===

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_gnome_focus_query_on_start_and_unpause() {
    with_test_timeout(async {
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");
        let (_focus_service, call_count) =
            start_gnome_focus_service(&address, "gnome-app", "Gnome Window").await;

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("gnome-app".to_string()),
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

        let client_connection = zbus::connection::Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        apply_focus_for_env(
            Environment::Gnome,
            Some(&client_connection),
            false,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        .expect("Failed to apply GNOME focus on startup");

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
            Environment::Gnome,
            Some(client_connection.clone()),
            false,
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

        let call_count = call_count.load(std::sync::atomic::Ordering::SeqCst);
        assert!(call_count >= 2, "expected focus query on start and unpause");
    })
    .await;
}
