use super::*;

// === X11/Xvfb Integration Tests ===

/// Check if Xvfb is available by trying to run it with -help
fn xvfb_available() -> bool {
    std::process::Command::new("Xvfb")
        .arg("-help")
        .output()
        .map(|_| true) // -help exits with 0 or 1 depending on version, but if it runs it's available
        .unwrap_or(false)
}

/// Guard struct that starts Xvfb and kills it on drop
struct XvfbGuard {
    child: std::process::Child,
    display: String,
}

impl XvfbGuard {
    /// Start Xvfb with a specific display number.
    /// Each test should use a unique hardcoded display number to allow parallel execution
    /// (nextest runs each test in a separate process).
    fn start(display_num: u32) -> Option<Self> {
        if !xvfb_available() {
            return None;
        }

        let display = format!(":{}", display_num);
        let child = std::process::Command::new("Xvfb")
            .args([&display, "-screen", "0", "800x600x24"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;

        // Wait for Xvfb to accept connections
        let display_clone = display.clone();
        wait_for(|| x11rb::connect(Some(&display_clone)).ok()).ok()?;

        Some(Self { child, display })
    }

    /// Connect to the Xvfb display with retry logic
    fn connect(&self) -> Result<(x11rb::rust_connection::RustConnection, usize), &'static str> {
        wait_for(|| x11rb::connect(Some(&self.display)).ok())
    }
}

impl Drop for XvfbGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn test_x11_state_display_override_ignores_stale_display_env() {
    let _display_env_lock = DISPLAY_ENV_LOCK.lock().unwrap();
    let xvfb = XvfbGuard::start(104)
        .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");
    let _stale_display = EnvVarGuard::set("DISPLAY", ":65535");

    let state = X11State::new(Some(&xvfb.display))
        .expect("X11State should connect using explicit display override");
    drop(state);
}

/// Test that X11State can connect to an X server and receive PropertyNotify events
///
/// Requires Xvfb. Run via `nix run .#test` or install Xvfb manually.
#[test]
fn test_x11_property_notify() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;
    use x11rb::wrapper::ConnectionExt as WrapperExt;

    // Fail if Xvfb is not available (display :100 for this test)
    let xvfb = XvfbGuard::start(100)
        .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");

    // Connect to Xvfb - "daemon" side that subscribes to PropertyNotify
    let (daemon_conn, screen) = xvfb.connect().expect("Failed to connect to Xvfb");
    let root = daemon_conn.setup().roots[screen].root;
    let atoms = X11Atoms::new(&daemon_conn)
        .expect("Failed to create atoms")
        .reply()
        .expect("Failed to get atoms");

    // Subscribe to PropertyNotify on root window
    daemon_conn
        .change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .expect("Failed to subscribe to events");
    daemon_conn.flush().expect("Failed to flush");

    // "App" side - creates window and triggers focus change
    let (app_conn, _) = xvfb.connect().expect("Failed to connect app to Xvfb");

    // Create a test window
    let win = app_conn
        .generate_id()
        .expect("Failed to generate window id");
    app_conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .expect("Failed to create window");

    // Set WM_CLASS: "instance\0TestApp\0"
    WrapperExt::change_property8(
        &app_conn,
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"instance\0TestApp\0",
    )
    .expect("Failed to set WM_CLASS");

    // Set _NET_WM_NAME
    WrapperExt::change_property8(
        &app_conn,
        PropMode::REPLACE,
        win,
        atoms._NET_WM_NAME,
        atoms.UTF8_STRING,
        b"Test Window Title",
    )
    .expect("Failed to set _NET_WM_NAME");

    // Simulate focus: set _NET_ACTIVE_WINDOW on root
    WrapperExt::change_property32(
        &app_conn,
        PropMode::REPLACE,
        root,
        atoms._NET_ACTIVE_WINDOW,
        AtomEnum::WINDOW,
        &[win],
    )
    .expect("Failed to set _NET_ACTIVE_WINDOW");
    app_conn.flush().expect("Failed to flush app connection");

    // Wait for PropertyNotify event to arrive
    let event = wait_for(|| daemon_conn.poll_for_event().ok().flatten())
        .expect("Timeout waiting for PropertyNotify event");

    match Some(event) {
        Some(x11rb::protocol::Event::PropertyNotify(e)) => {
            assert_eq!(
                e.atom, atoms._NET_ACTIVE_WINDOW,
                "Expected _NET_ACTIVE_WINDOW property change"
            );

            // Now verify we can read the window info using X11State logic
            // Get the active window ID
            let prop_reply = daemon_conn
                .get_property(
                    false,
                    root,
                    atoms._NET_ACTIVE_WINDOW,
                    AtomEnum::WINDOW,
                    0,
                    1,
                )
                .expect("Failed to get property")
                .reply()
                .expect("Failed to get property reply");

            assert!(
                prop_reply.value.len() >= 4,
                "Expected window ID in property"
            );
            let arr: [u8; 4] = prop_reply.value[..4].try_into().unwrap();
            let active_win = u32::from_le_bytes(arr);
            assert_eq!(active_win, win, "Active window should be our test window");

            // Read WM_CLASS
            let class_reply = daemon_conn
                .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
                .expect("Failed to get WM_CLASS")
                .reply()
                .expect("Failed to get WM_CLASS reply");

            // Parse WM_CLASS format: "instance\0class\0"
            let parts: Vec<&[u8]> = class_reply.value.split(|&b| b == 0).collect();
            assert!(parts.len() >= 2, "Expected instance and class in WM_CLASS");
            let class = String::from_utf8_lossy(parts[1]);
            assert_eq!(class, "TestApp", "Window class should be TestApp");

            // Read _NET_WM_NAME
            let title_reply = daemon_conn
                .get_property(false, win, atoms._NET_WM_NAME, atoms.UTF8_STRING, 0, 1024)
                .expect("Failed to get _NET_WM_NAME")
                .reply()
                .expect("Failed to get _NET_WM_NAME reply");

            let title = String::from_utf8_lossy(&title_reply.value);
            assert_eq!(title, "Test Window Title", "Window title should match");
        }
        Some(other) => {
            panic!("Expected PropertyNotify event, got {:?}", other);
        }
        None => {
            panic!("No event received - PropertyNotify was not triggered");
        }
    }
}

/// Test X11State integration with FocusHandler
///
/// This tests the full flow: X11 events → X11State → FocusHandler → actions
/// Requires Xvfb. Run via `nix run .#test` or install Xvfb manually.
#[test]
fn test_x11_focus_handler_integration() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;
    use x11rb::wrapper::ConnectionExt as WrapperExt;

    // Display :101 for this test
    let xvfb = XvfbGuard::start(101)
        .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");

    // Set up X11 connections
    let (conn, screen) = xvfb.connect().expect("Failed to connect");
    let root = conn.setup().roots[screen].root;
    let atoms = X11Atoms::new(&conn).unwrap().reply().unwrap();

    // Create X11State
    let x11_state = X11State {
        connection: conn,
        root,
        atoms,
    };

    // Create FocusHandler with test rules
    let rules = vec![Rule {
        class: Some("TestApp".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("test-layer".to_string()),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    // Prime the handler with initial state (no active window)
    // This sets up last_window so subsequent calls detect changes correctly
    let info = x11_state.get_active_window();
    let _ = handler.handle(&info, "default");

    // Create app connection and window
    let (app_conn, _) = xvfb.connect().expect("Failed to connect app");
    let win = app_conn.generate_id().unwrap();
    app_conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .unwrap();

    // Set WM_CLASS to "instance\0TestApp\0"
    WrapperExt::change_property8(
        &app_conn,
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"instance\0TestApp\0",
    )
    .unwrap();

    // Set _NET_ACTIVE_WINDOW
    WrapperExt::change_property32(
        &app_conn,
        PropMode::REPLACE,
        root,
        x11_state.atoms._NET_ACTIVE_WINDOW,
        AtomEnum::WINDOW,
        &[win],
    )
    .unwrap();
    app_conn.sync().unwrap(); // Ensure server processed the property change

    // Now get active window and handle focus
    let info = x11_state.get_active_window();
    assert_eq!(info.class, "TestApp", "Should detect TestApp window class");

    let actions = handler.handle(&info, "default");
    assert!(actions.is_some());
    let actions = actions.unwrap();
    assert!(
        actions
            .actions
            .contains(&FocusAction::ChangeLayer("test-layer".to_string()))
    );
}

/// Test that multiple focus changes are tracked correctly
/// Requires Xvfb. Run via `nix run .#test` or install Xvfb manually.
#[test]
fn test_x11_multiple_focus_changes() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;
    use x11rb::wrapper::ConnectionExt as WrapperExt;

    // Display :102 for this test
    let xvfb = XvfbGuard::start(102)
        .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");

    let (conn, screen) = xvfb.connect().expect("Failed to connect");
    let root = conn.setup().roots[screen].root;
    let atoms = X11Atoms::new(&conn).unwrap().reply().unwrap();

    let x11_state = X11State {
        connection: conn,
        root,
        atoms,
    };

    let rules = vec![
        Rule {
            class: Some("App1".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer1".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
        Rule {
            class: Some("App2".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer2".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Skip initial empty state
    handler.handle(&x11_state.get_active_window(), "default");

    let (app_conn, _) = xvfb.connect().unwrap();

    // Create first window (App1)
    let win1 = app_conn.generate_id().unwrap();
    app_conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win1,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .unwrap();
    WrapperExt::change_property8(
        &app_conn,
        PropMode::REPLACE,
        win1,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"instance\0App1\0",
    )
    .unwrap();

    // Create second window (App2)
    let win2 = app_conn.generate_id().unwrap();
    app_conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win2,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .unwrap();
    WrapperExt::change_property8(
        &app_conn,
        PropMode::REPLACE,
        win2,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"instance\0App2\0",
    )
    .unwrap();

    // Focus App1
    WrapperExt::change_property32(
        &app_conn,
        PropMode::REPLACE,
        root,
        x11_state.atoms._NET_ACTIVE_WINDOW,
        AtomEnum::WINDOW,
        &[win1],
    )
    .unwrap();
    app_conn.sync().unwrap();

    let info = x11_state.get_active_window();
    assert_eq!(info.class, "App1");
    let actions = handler.handle(&info, "default").unwrap();
    assert!(
        actions
            .actions
            .contains(&FocusAction::ChangeLayer("layer1".to_string()))
    );

    // Focus App2
    WrapperExt::change_property32(
        &app_conn,
        PropMode::REPLACE,
        root,
        x11_state.atoms._NET_ACTIVE_WINDOW,
        AtomEnum::WINDOW,
        &[win2],
    )
    .unwrap();
    app_conn.sync().unwrap();

    let info = x11_state.get_active_window();
    assert_eq!(info.class, "App2");
    let actions = handler.handle(&info, "default").unwrap();
    assert!(
        actions
            .actions
            .contains(&FocusAction::ChangeLayer("layer2".to_string()))
    );

    // Focus nothing (unfocus)
    WrapperExt::change_property32(
        &app_conn,
        PropMode::REPLACE,
        root,
        x11_state.atoms._NET_ACTIVE_WINDOW,
        AtomEnum::WINDOW,
        &[0u32],
    )
    .unwrap();
    app_conn.sync().unwrap();

    let info = x11_state.get_active_window();
    assert_eq!(info.class, "");
    let actions = handler.handle(&info, "default").unwrap();
    assert!(
        actions
            .actions
            .contains(&FocusAction::ChangeLayer("default".to_string()))
    );
}

/// Test that the daemon queries focused window on startup and unpause (X11).
/// Requires Xvfb. Run via `nix run .#test` or install Xvfb manually.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_x11_focus_query_on_start_and_unpause() {
    with_long_test_timeout(async {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::*;
        use x11rb::wrapper::ConnectionExt as WrapperExt;
        let _display_env_lock = DISPLAY_ENV_LOCK.lock().unwrap();
        let _x11_focus_query_lock = X11_FOCUS_QUERY_LOCK.lock().unwrap();

        let xvfb = XvfbGuard::start(103)
            .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");
        let _display = EnvVarGuard::set("DISPLAY", &xvfb.display);

        let (conn, screen) = xvfb.connect().expect("Failed to connect");
        let root = conn.setup().roots[screen].root;
        let atoms = X11Atoms::new(&conn).unwrap().reply().unwrap();

        let win = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .unwrap();

        WrapperExt::change_property8(
            &conn,
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"instance\0X11App\0",
        )
        .unwrap();
        WrapperExt::change_property32(
            &conn,
            PropMode::REPLACE,
            root,
            atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            &[win],
        )
        .unwrap();
        conn.flush().unwrap();

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("X11App".to_string()),
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

        apply_focus_for_env(
            Environment::X11,
            None,
            false,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        .expect("Failed to apply X11 focus on startup");

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
            Environment::X11,
            None,
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
    })
    .await;
}

/// Regression: unpause-time X11 focus refresh must use runtime display override, not stale env.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_x11_unpause_focus_query_uses_runtime_display_override() {
    with_long_test_timeout(async {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::*;
        use x11rb::wrapper::ConnectionExt as WrapperExt;
        let _display_env_lock = DISPLAY_ENV_LOCK.lock().unwrap();
        let _x11_focus_query_lock = X11_FOCUS_QUERY_LOCK.lock().unwrap();

        let xvfb = XvfbGuard::start(105)
            .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");
        let _display = EnvVarGuard::set("DISPLAY", &xvfb.display);
        let _x11_override =
            super::set_test_focus_query_display_override(Environment::X11, Some(&xvfb.display));

        let (conn, screen) = xvfb.connect().expect("Failed to connect");
        let root = conn.setup().roots[screen].root;
        let atoms = X11Atoms::new(&conn).unwrap().reply().unwrap();

        let win = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::default(),
        )
        .unwrap();

        WrapperExt::change_property8(
            &conn,
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"instance\0X11AppOverride\0",
        )
        .unwrap();
        WrapperExt::change_property32(
            &conn,
            PropMode::REPLACE,
            root,
            atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            &[win],
        )
        .unwrap();
        conn.flush().unwrap();

        let mock_server = MockKanataServer::start();
        let rules = vec![Rule {
            class: Some("X11AppOverride".to_string()),
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

        apply_focus_for_env(
            Environment::X11,
            None,
            false,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        .expect("Failed to apply X11 focus on startup");

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

        let _stale_display = EnvVarGuard::set("DISPLAY", ":65535");

        unpause_daemon_direct(
            Environment::X11,
            None,
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
    })
    .await;
}
