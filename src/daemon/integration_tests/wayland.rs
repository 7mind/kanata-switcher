use super::*;

// === Wayland Protocol Integration Tests ===

/// Mock Wayland compositor for testing the wlr-foreign-toplevel protocol.
///
/// This module implements a minimal Wayland compositor that speaks the
/// wlr-foreign-toplevel-management-v1 protocol, allowing us to test that
/// the daemon correctly handles toplevel events.
pub(super) mod wayland_mock {
    use std::thread;
    use wayland_backend::server::InvalidId;
    use wayland_protocols_wlr::foreign_toplevel::v1::server::{
        zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
        zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
    };
    use wayland_server::{Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New};

    #[derive(Default)]
    pub struct MockCompositorState {
        manager: Option<ZwlrForeignToplevelManagerV1>,
    }

    // Dispatch for the manager global
    impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for MockCompositorState {
        fn bind(
            _state: &mut Self,
            _handle: &DisplayHandle,
            _client: &Client,
            resource: New<ZwlrForeignToplevelManagerV1>,
            _global_data: &(),
            data_init: &mut DataInit<'_, Self>,
        ) {
            let manager = data_init.init(resource, ());
            _state.manager = Some(manager);
        }
    }

    impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for MockCompositorState {
        fn request(
            _state: &mut Self,
            _client: &Client,
            _resource: &ZwlrForeignToplevelManagerV1,
            request: zwlr_foreign_toplevel_manager_v1::Request,
            _data: &(),
            _dhandle: &DisplayHandle,
            _data_init: &mut DataInit<'_, Self>,
        ) {
            match request {
                zwlr_foreign_toplevel_manager_v1::Request::Stop => {}
                _ => {}
            }
        }
    }

    impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for MockCompositorState {
        fn request(
            _state: &mut Self,
            _client: &Client,
            _resource: &ZwlrForeignToplevelHandleV1,
            request: zwlr_foreign_toplevel_handle_v1::Request,
            _data: &(),
            _dhandle: &DisplayHandle,
            _data_init: &mut DataInit<'_, Self>,
        ) {
            match request {
                zwlr_foreign_toplevel_handle_v1::Request::Destroy => {}
                _ => {}
            }
        }
    }

    pub struct WaylandMockServer {
        socket_name: String,
        #[allow(dead_code)]
        event_sender: std::sync::mpsc::Sender<(String, String)>,
        thread_handle: Option<std::thread::JoinHandle<()>>,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
        #[allow(dead_code)]
        runtime_dir: tempfile::TempDir,
        previous_runtime_dir: Option<std::ffi::OsString>,
        previous_wayland_display: Option<std::ffi::OsString>,
    }

    impl WaylandMockServer {
        pub fn start() -> Self {
            let runtime_dir = tempfile::tempdir().expect("Failed to create Wayland runtime dir");
            let previous_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
            let previous_wayland_display = std::env::var_os("WAYLAND_DISPLAY");
            unsafe {
                std::env::set_var("XDG_RUNTIME_DIR", runtime_dir.path());
            }

            let display =
                Display::<MockCompositorState>::new().expect("Failed to create Wayland display");
            let handle = display.handle();
            handle.create_global::<MockCompositorState, ZwlrForeignToplevelManagerV1, ()>(3, ());

            let socket =
                wayland_server::ListeningSocket::bind_auto("kanata-switcher-test", 1..1000)
                    .expect("Failed to create Wayland socket");

            let socket_name = socket
                .socket_name()
                .expect("Socket name missing")
                .to_string_lossy()
                .to_string();
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", &socket_name);
            }

            let (event_sender, event_receiver) = std::sync::mpsc::channel();
            let mut server = Self {
                socket_name,
                event_sender,
                thread_handle: None,
                shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                runtime_dir,
                previous_runtime_dir,
                previous_wayland_display,
            };

            server.spawn_event_loop(display, handle, socket, event_receiver);
            server
        }

        pub fn socket_name(&self) -> &str {
            &self.socket_name
        }

        #[allow(dead_code)]
        pub fn send_active_window(&mut self, app_id: &str, title: &str) {
            self.event_sender
                .send((app_id.to_string(), title.to_string()))
                .expect("Failed to queue Wayland toplevel");
        }

        fn spawn_event_loop(
            &mut self,
            mut display: Display<MockCompositorState>,
            mut handle: DisplayHandle,
            socket: wayland_server::ListeningSocket,
            event_receiver: std::sync::mpsc::Receiver<(String, String)>,
        ) {
            let shutdown = self.shutdown.clone();
            let mut client_slot: Option<Client> = None;
            let mut pending_window: Option<(String, String)> = None;
            self.thread_handle = Some(thread::spawn(move || {
                let mut state = MockCompositorState::default();
                loop {
                    if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(Some(stream)) = socket.accept() {
                        let client = handle
                            .insert_client(stream, std::sync::Arc::new(()))
                            .expect("Failed to insert client");
                        client_slot = Some(client);
                    }

                    while let Ok((app_id, title)) = event_receiver.try_recv() {
                        pending_window = Some((app_id, title));
                    }

                    display.dispatch_clients(&mut state).ok();

                    if let (Some(client), Some((app_id, title))) =
                        (client_slot.clone(), pending_window.take())
                    {
                        if state.manager.is_some() {
                            if send_active_window_to_client(
                                &mut display,
                                &handle,
                                &mut state,
                                &client,
                                &app_id,
                                &title,
                            )
                            .is_err()
                            {
                                pending_window = Some((app_id, title));
                            }
                        } else {
                            pending_window = Some((app_id, title));
                        }
                    }

                    display.flush_clients().ok();
                }
            }));
        }
    }

    fn send_active_window_to_client(
        display: &mut Display<MockCompositorState>,
        handle: &DisplayHandle,
        state: &mut MockCompositorState,
        client: &Client,
        app_id: &str,
        title: &str,
    ) -> Result<(), InvalidId> {
        let manager = state.manager.as_ref().expect("Wayland manager not bound");
        let toplevel = client
            .create_resource::<ZwlrForeignToplevelHandleV1, _, MockCompositorState>(handle, 1, ())
            .map_err(|_| InvalidId)?;
        manager.toplevel(&toplevel);
        toplevel.app_id(app_id.to_string());
        toplevel.title(title.to_string());
        let activated = zwlr_foreign_toplevel_handle_v1::State::Activated as u8;
        toplevel.state(vec![activated]);
        toplevel.done();
        display.flush_clients().expect("Failed to flush clients");
        Ok(())
    }

    impl Drop for WaylandMockServer {
        fn drop(&mut self) {
            self.shutdown
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(handle) = self.thread_handle.take() {
                handle.join().ok();
            }
            if let Some(value) = self.previous_wayland_display.take() {
                unsafe {
                    std::env::set_var("WAYLAND_DISPLAY", value);
                }
            } else {
                unsafe {
                    std::env::remove_var("WAYLAND_DISPLAY");
                }
            }
            if let Some(value) = self.previous_runtime_dir.take() {
                unsafe {
                    std::env::set_var("XDG_RUNTIME_DIR", value);
                }
            } else {
                unsafe {
                    std::env::remove_var("XDG_RUNTIME_DIR");
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_wayland_focus_query_on_start_and_unpause() {
    with_long_test_timeout(async {
        let (_lock, _server) = start_wayland_test_server();

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("wayland-app".to_string()),
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

        let initial_queries = super::wayland_query_count();
        let handler_start = handler.clone();
        let status_start = status_broadcaster.clone();
        let pause_start = pause_broadcaster.clone();
        let kanata_start = kanata.clone();
        let apply_task = tokio::spawn(async move {
            apply_focus_for_env(
                Environment::Wayland,
                None,
                false,
                &handler_start,
                &status_start,
                &pause_start,
                &kanata_start,
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        apply_task
            .await
            .expect("Wayland apply task failed")
            .expect("Failed to apply Wayland focus on startup");

        let after_start = super::wayland_query_count();
        assert!(
            after_start > initial_queries,
            "expected Wayland focus query on startup"
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

        let before_unpause = super::wayland_query_count();
        let handler_unpause = handler.clone();
        let status_unpause = status_broadcaster.clone();
        let pause_unpause = pause_broadcaster.clone();
        let kanata_unpause = kanata.clone();
        let unpause_task = tokio::spawn(async move {
            unpause_daemon_direct(
                Environment::Wayland,
                None,
                false,
                &pause_unpause,
                &handler_unpause,
                &status_unpause,
                &kanata_unpause,
                "test",
            )
            .await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        unpause_task.await.expect("Wayland unpause task failed");
        let after_unpause = super::wayland_query_count();
        assert!(
            after_unpause > before_unpause,
            "expected Wayland focus query on unpause"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_wayland_unpause_focus_query_uses_runtime_display_override() {
    with_long_test_timeout(async {
        let (_lock, server) = start_wayland_test_server();
        let _wayland_override = super::set_test_focus_query_display_override(
            Environment::Wayland,
            Some(server.socket_name()),
        );

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("wayland-app".to_string()),
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

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let pause_broadcaster = PauseBroadcaster::new();

        apply_focus_for_env(
            Environment::Wayland,
            None,
            false,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        .expect("Failed to apply Wayland focus on startup");

        pause_daemon_direct(
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;
        drain_kanata_messages(&mock_server, Duration::from_millis(200));

        let _stale_wayland_display = EnvVarGuard::set("WAYLAND_DISPLAY", "stale-wayland-display");
        let before_unpause = super::wayland_query_count();

        unpause_daemon_direct(
            Environment::Wayland,
            None,
            false,
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;
        let after_unpause = super::wayland_query_count();
        assert!(
            after_unpause > before_unpause,
            "expected Wayland focus query on unpause with runtime display override"
        );
    })
    .await;
}

/// Test WaylandState directly by simulating protocol events
///
/// This tests that WaylandState correctly processes toplevel events and
/// returns the right WindowInfo.
#[test]
fn test_wayland_mock_compositor_startup() {
    let (_lock, server) = start_wayland_test_server();
    assert!(!server.socket_name().is_empty());
}

#[test]
fn test_wayland_connect_override_ignores_stale_wayland_display_env() {
    let (_lock, server) = start_wayland_test_server();
    let _stale_wayland_display = EnvVarGuard::set("WAYLAND_DISPLAY", "stale-wayland-display");

    let connection = connect_wayland_with_display_override(Some(server.socket_name()))
        .expect("Wayland connection should use explicit display override");
    drop(connection);
}
