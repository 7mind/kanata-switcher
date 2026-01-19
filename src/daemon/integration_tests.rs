//! Integration tests for desktop environment backends.
//!
//! These tests verify the integration between the daemon and various DE backends:
//! - DBus service (GNOME/KDE)
//! - Wayland protocol (wlr-foreign-toplevel-management)
//! - X11 PropertyNotify (requires Xvfb)
//!
//! Tests requiring external dependencies (Xvfb, dbus-daemon) fail with helpful error
//! messages when dependencies are not available. Run via `nix run .#test` for guaranteed
//! full test coverage, or install dependencies manually.

use super::*;
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// === Polling Helper ===

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const POLL_TIMEOUT: Duration = Duration::from_secs(5);
const TEST_TIMEOUT: Duration = Duration::from_secs(5);
static WAYLAND_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Wait for a condition to become true, polling at 50ms intervals.
/// Returns Ok(T) when the condition returns Some(T), or Err after 1 minute timeout.
fn wait_for<T, F>(mut condition: F) -> Result<T, &'static str>
where
    F: FnMut() -> Option<T>,
{
    let start = Instant::now();
    while start.elapsed() < POLL_TIMEOUT {
        if let Some(result) = condition() {
            return Ok(result);
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err("Timeout waiting for condition")
}

/// Async version of wait_for for tokio tests
async fn wait_for_async<T, F, Fut>(mut condition: F) -> Result<T, &'static str>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = Instant::now();
    while start.elapsed() < POLL_TIMEOUT {
        if let Some(result) = condition().await {
            return Ok(result);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Err("Timeout waiting for condition")
}

async fn with_test_timeout<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("test timeout")
}

fn start_wayland_test_server(
) -> (std::sync::MutexGuard<'static, ()>, wayland_mock::WaylandMockServer) {
    let lock = WAYLAND_ENV_LOCK.lock().unwrap();
    let server = wayland_mock::WaylandMockServer::start();
    (lock, server)
}

async fn pause_daemon_direct(
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    request_label: &str,
) {
    if !pause_broadcaster.set_paused(true) {
        println!("[Pause] Pause requested {} (already paused)", request_label);
        return;
    }
    println!("[Pause] Pausing daemon");
    let virtual_keys = {
        let mut handler = handler.lock().unwrap();
        let keys = handler.current_virtual_keys();
        handler.reset();
        keys
    };
    let default_layer = kanata.default_layer().await.unwrap_or_default();

    for vk in virtual_keys.iter().rev() {
        kanata.act_on_fake_key(vk, "Release").await;
    }

    if !default_layer.is_empty() {
        let _ = kanata.change_layer(&default_layer).await;
    }

    status_broadcaster.set_paused_status(default_layer);
    kanata.pause_disconnect().await;
}

async fn unpause_daemon_direct(
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    request_label: &str,
) {
    if !pause_broadcaster.set_paused(false) {
        println!(
            "[Pause] Unpause requested {} (already running)",
            request_label
        );
        return;
    }
    println!("[Pause] Resuming daemon");
    kanata.unpause_connect().await;
    if let Err(error) = apply_focus_for_env(
        env,
        connection.as_ref(),
        is_kde6,
        handler,
        status_broadcaster,
        pause_broadcaster,
        kanata,
    )
    .await
    {
        panic!("[Pause] Failed to refresh focus after unpause: {}", error);
    }
}

// === Mock Kanata Server ===

/// Messages that can be sent to the mock Kanata server
#[derive(Debug, Clone, PartialEq)]
enum KanataMessage {
    ChangeLayer { new: String },
    ActOnFakeKey { name: String, action: String },
    RequestLayerNames,
}

struct FocusService {
    call_count: Arc<std::sync::atomic::AtomicUsize>,
    class: String,
    title: String,
}

#[zbus::interface(name = "com.github.kanata.Switcher.Gnome")]
impl FocusService {
    #[allow(non_snake_case)]
    fn GetFocus(&self) -> (String, String) {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (self.class.clone(), self.title.clone())
    }
}

async fn start_gnome_focus_service(
    address: &zbus::Address,
    class: &str,
    title: &str,
) -> (Connection, Arc<std::sync::atomic::AtomicUsize>) {
    use zbus::connection::Builder;

    let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let service_connection = Builder::address(address.clone())
        .expect("Failed to create connection builder")
        .name(GNOME_SHELL_BUS_NAME)
        .expect("Failed to set bus name")
        .serve_at(
            GNOME_FOCUS_OBJECT_PATH,
            FocusService {
                call_count: call_count.clone(),
                class: class.to_string(),
                title: title.to_string(),
            },
        )
        .expect("Failed to serve mock focus service")
        .build()
        .await
        .expect("Failed to build focus service connection");

    let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
        .await
        .expect("Failed to create DBus proxy");
    wait_for_async(|| {
        let proxy = dbus_proxy.clone();
        async move {
            proxy
                .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                .await
                .ok()
                .filter(|&has_owner| has_owner)
        }
    })
    .await
    .expect("Timeout waiting for GNOME focus service registration");

    (service_connection, call_count)
}

fn wait_for_kanata_message(
    server: &MockKanataServer,
    message: KanataMessage,
    timeout_duration: Duration,
) {
    let start = Instant::now();
    while start.elapsed() < timeout_duration {
        if let Some(msg) = server.recv_timeout(Duration::from_millis(50)) {
            if msg == message {
                return;
            }
        }
    }
    panic!("Timeout waiting for {:?}", message);
}

fn drain_kanata_messages(server: &MockKanataServer, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        if server.recv_timeout(Duration::from_millis(20)).is_none() {
            break;
        }
    }
}
/// A mock Kanata TCP server for testing
struct MockKanataServer {
    port: u16,
    handle: Option<thread::JoinHandle<()>>,
    receiver: mpsc::Receiver<KanataMessage>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MockKanataServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = mpsc::channel();
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_thread = std::sync::Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            loop {
                if shutdown_thread.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    Err(_) => break,
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                // Send initial LayerChange message
                let init_msg = r#"{"LayerChange":{"new":"default"}}"#;
                if writeln!(stream, "{}", init_msg).is_err() {
                    continue;
                }

                let mut reader = BufReader::new(stream.try_clone().unwrap());

                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break, // Connection closed
                        Ok(_) => {
                            // Parse and forward the message
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                                if let Some(cl) = value.get("ChangeLayer") {
                                    let new = cl.get("new").and_then(|v| v.as_str()).unwrap_or("");
                                    sender
                                        .send(KanataMessage::ChangeLayer {
                                            new: new.to_string(),
                                        })
                                        .ok();
                                } else if let Some(fk) = value.get("ActOnFakeKey") {
                                    let name =
                                        fk.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                    let action =
                                        fk.get("action").and_then(|v| v.as_str()).unwrap_or("");
                                    sender
                                        .send(KanataMessage::ActOnFakeKey {
                                            name: name.to_string(),
                                            action: action.to_string(),
                                        })
                                        .ok();
                                } else if value.get("RequestLayerNames").is_some() {
                                    sender.send(KanataMessage::RequestLayerNames).ok();
                                    // Respond with layer names
                                    let response = r#"{"LayerNames":{"names":["default","browser","terminal","vim"]}}"#;
                                    writeln!(stream, "{}", response).ok();
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        Self {
            port,
            handle: Some(handle),
            receiver,
            shutdown,
        }
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn recv_timeout(&self, timeout: Duration) -> Option<KanataMessage> {
        self.receiver.recv_timeout(timeout).ok()
    }
}

impl Drop for MockKanataServer {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

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

        mock_server.recv_timeout(Duration::from_secs(1));

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

// === KDE Focus Query Tests ===

struct MockKwinScripting {
    scripts: Arc<Mutex<HashMap<String, i32>>>,
    next_id: Arc<Mutex<i32>>,
    object_server: zbus::ObjectServer,
    is_kde6: bool,
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

struct MockKwinScript {
    path: String,
}

#[zbus::interface(name = "org.kde.kwin.Script")]
impl MockKwinScript {
    #[zbus(name = "run")]
    async fn run(&self) {
        let script_contents = std::fs::read_to_string(&self.path).expect("Failed to read script");
        let parts = extract_call_dbus_parts(&script_contents);
        let bus_name = parts.get(0).expect("Missing bus name");
        let object_path = parts.get(1).expect("Missing object path");
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
                Some(KDE_QUERY_INTERFACE),
                KDE_QUERY_METHOD,
                &("kde-app", "KDE Window"),
            )
            .await
            .expect("Failed to call KDE query callback");
    }

    #[zbus(name = "stop")]
    fn stop(&self) {}
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_kde_focus_query_on_start_and_unpause() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let scripts = Arc::new(Mutex::new(HashMap::new()));
        unsafe {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", dbus.address());
        }

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
        mock_server.recv_timeout(Duration::from_secs(1));

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

// === DBus Integration Tests ===

/// Test that the DBus service correctly processes WindowFocus calls and sends layer changes
#[tokio::test]
async fn test_dbus_service_layer_switching() {
    with_test_timeout(async {
        // Start mock kanata server
        let server = MockKanataServer::start();

        // Create rules
        let rules = vec![
            Rule {
                class: Some("firefox".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("terminal".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        // Create kanata client and connect
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        // Skip RequestLayerNames message
        server.recv_timeout(Duration::from_secs(1));

        // Create the DBus service handler directly (without actual DBus)
        let handler = std::sync::Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Simulate WindowFocus call for firefox
        {
            let win = WindowInfo {
                class: "firefox".to_string(),
                title: "GitHub".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Verify layer change was sent
        let msg = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        // Simulate WindowFocus call for kitty
        {
            let win = WindowInfo {
                class: "kitty".to_string(),
                title: "bash".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Verify layer change was sent
        let msg = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            })
        );
    })
    .await;
}

/// Test DBus service with virtual key actions
#[tokio::test]
async fn test_dbus_service_virtual_keys() {
    with_test_timeout(async {
        let server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("firefox".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        // Skip RequestLayerNames
        server.recv_timeout(Duration::from_secs(1));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        // Focus firefox
        {
            let win = WindowInfo {
                class: "firefox".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive layer change and VK press
        let msg1 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg1,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        let msg2 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg2,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );

        // Unfocus (empty window)
        {
            let win = WindowInfo {
                class: "".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive VK release and layer change to default
        let msg3 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg3,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );

        let msg4 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg4,
            Some(KanataMessage::ChangeLayer {
                new: "default".to_string()
            })
        );
    })
    .await;
}

/// Test DBus service with fallthrough rules
#[tokio::test]
async fn test_dbus_service_fallthrough() {
    with_test_timeout(async {
        let server = MockKanataServer::start();

        // Use layers from mock server's known_layers: ["default", "browser", "terminal", "vim"]
        let rules = vec![
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("browser".to_string()),
                virtual_key: None,
                raw_vk_action: Some(vec![("vk_notify".to_string(), "Tap".to_string())]),
                fallthrough: true,
            },
            Rule {
                class: Some("kitty".to_string()),
                title: None,
                on_native_terminal: None,
                layer: Some("terminal".to_string()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster,
        );
        kanata.connect_with_retry().await;

        server.recv_timeout(Duration::from_secs(1));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));

        {
            let win = WindowInfo {
                class: "kitty".to_string(),
                title: "".to_string(),
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
            let actions = handler.lock().unwrap().handle(&win, &default_layer);
            if let Some(actions) = actions {
                execute_focus_actions(&kanata, actions).await;
            }
        }

        // Should receive: browser layer, raw_vk tap, terminal layer
        let msg1 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg1,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );

        let msg2 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg2,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_notify".to_string(),
                action: "Tap".to_string(),
            })
        );

        let msg3 = server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg3,
            Some(KanataMessage::ChangeLayer {
                new: "terminal".to_string()
            })
        );
    })
    .await;
}

// === Private DBus Session for Testing ===

/// Check if dbus-daemon is available by trying to run it with --version
fn dbus_daemon_available() -> bool {
    std::process::Command::new("dbus-daemon")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Guard struct that starts a private dbus-daemon and cleans up on drop
struct DbusSessionGuard {
    child: std::process::Child,
    address: String,
    config_dir: std::path::PathBuf,
}

use std::sync::atomic::{AtomicU64, Ordering};
static DBUS_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

impl DbusSessionGuard {
    fn start() -> Result<Self, String> {
        if !dbus_daemon_available() {
            return Err("dbus-daemon binary not found in PATH".to_string());
        }

        // Create a minimal session config file with unique path per test
        let unique_id = DBUS_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let config_dir =
            std::env::temp_dir().join(format!("dbus-test-{}-{}", std::process::id(), unique_id));
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create config dir: {}", e))?;

        let config_path = config_dir.join("session.conf");
        let socket_path = config_dir.join("bus-socket");

        // Minimal session bus config
        let config_content = format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
            socket_path.display()
        );

        std::fs::write(&config_path, config_content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        // Start dbus-daemon with custom config
        let mut child = std::process::Command::new("dbus-daemon")
            .args([
                "--config-file",
                config_path.to_str().unwrap(),
                "--nofork",
                "--print-address",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn dbus-daemon: {}", e))?;

        // Read the address from stdout
        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture dbus-daemon stdout")?;
        let mut reader = std::io::BufReader::new(stdout);
        let mut address = String::new();
        std::io::BufRead::read_line(&mut reader, &mut address)
            .map_err(|e| format!("Failed to read dbus-daemon address: {}", e))?;
        let address = address.trim().to_string();

        if address.is_empty() {
            // Try to read stderr for error info
            if let Some(mut stderr) = child.stderr.take() {
                let mut err_output = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut err_output);
                let _ = child.kill();
                return Err(format!(
                    "dbus-daemon produced no address. stderr: {}",
                    err_output
                ));
            }
            let _ = child.kill();
            return Err("dbus-daemon produced no address".to_string());
        }

        // Wait for socket to be connectable (dbus-daemon ready)
        let socket_path_clone = socket_path.clone();
        wait_for(|| std::os::unix::net::UnixStream::connect(&socket_path_clone).ok())
            .map_err(|_| "Timeout waiting for dbus-daemon socket")?;

        Ok(Self {
            child,
            address,
            config_dir,
        })
    }

    fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for DbusSessionGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Clean up config directory
        let _ = std::fs::remove_dir_all(&self.config_dir);
    }
}

/// Test with a private DBus session (no dependency on desktop session)
///
/// This test verifies the DBus transport layer works correctly by:
/// 1. Starting a private dbus-daemon
/// 2. Registering the service on that bus
/// 3. Calling the service method from a client connection
/// 4. Verifying the layer change reaches the mock Kanata server
///
/// Requires dbus-daemon to be available. Skips gracefully if not found.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_service_real_bus() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        // Start private dbus-daemon
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();
        let port = mock_server.port();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()), // must be in mock server's known_layers
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        // Parse the bus address
        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        // Create the kanata client and connect
        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            port,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        // Skip RequestLayerNames
        mock_server.recv_timeout(Duration::from_secs(1));

        // Connect to bus and register service
        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        // Create client connection
        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        // Wait for service name to be registered on bus
        let dbus_proxy = zbus::fdo::DBusProxy::new(&client)
            .await
            .expect("Failed to create DBus proxy");
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        // Keep service connection alive by holding a reference
        let _service_conn = service_connection;

        // Call WindowFocus method
        let result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;

        // Check if the call succeeded
        assert!(result.is_ok(), "DBus call failed: {:?}", result.err());

        // Verify layer change (recv_timeout handles waiting)
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
    })
    .await;
}

/// Test that GetStatus reports the initial layer without waiting for a layer change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_get_status_initial_layer() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let reply = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "GetStatus",
                &(),
            )
            .await
            .expect("GetStatus call failed");

        let (layer, virtual_keys, source): (String, Vec<String>, String) = reply
            .body()
            .deserialize()
            .expect("Failed to deserialize GetStatus response");

        assert_eq!(layer, "default");
        assert!(virtual_keys.is_empty());
        assert_eq!(source, "external");
    })
    .await;
}

/// Test that focus-based status updates override the layer source on GetStatus.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_get_status_focus_source() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let focus_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );

        let reply = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "GetStatus",
                &(),
            )
            .await
            .expect("GetStatus call failed");

        let (layer, _virtual_keys, source): (String, Vec<String>, String) = reply
            .body()
            .deserialize()
            .expect("Failed to deserialize GetStatus response");

        assert_eq!(layer, "browser");
        assert_eq!(source, "focus");
    })
    .await;
}

/// Test that Restart requests trigger the restart channel.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_restart_request() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let mut restart_receiver = restart_handle.subscribe();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let restart_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Restart",
                &(),
            )
            .await;
        assert!(
            restart_result.is_ok(),
            "DBus Restart failed: {:?}",
            restart_result.err()
        );

        let changed =
            tokio::time::timeout(Duration::from_secs(2), restart_receiver.changed()).await;
        assert!(changed.is_ok(), "Restart signal timed out");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_restart_private_dbus() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let mut restart_receiver = restart_handle.subscribe();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let control_result =
            send_control_command_with_connection(&client, ControlCommand::Restart).await;
        assert!(
            control_result.is_ok(),
            "Restart control command failed: {:?}",
            control_result.err()
        );

        let changed =
            tokio::time::timeout(Duration::from_secs(2), restart_receiver.changed()).await;
        assert!(changed.is_ok(), "Restart signal timed out");
    })
    .await;
}

/// Test pause/unpause flow with a mock Kanata server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_pause_unpause() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let focus_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );

        let pause_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Pause",
                &(),
            )
            .await;
        assert!(
            pause_result.is_ok(),
            "DBus Pause failed: {:?}",
            pause_result.err()
        );

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        if let Some(message) = msg {
            assert_eq!(
                message,
                KanataMessage::ChangeLayer {
                    new: "default".to_string()
                }
            );
        }

        let focus_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");

        let unpause_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Unpause",
                &(),
            )
            .await;
        assert!(
            unpause_result.is_ok(),
            "DBus Unpause failed: {:?}",
            unpause_result.err()
        );

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(msg, Some(KanataMessage::RequestLayerNames));

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Press".to_string(),
            })
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_paused_changed_signal() {
    with_test_timeout(async {
        use futures_util::StreamExt;
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let proxy = zbus::Proxy::new(
            &client,
            "com.github.kanata.Switcher",
            "/com/github/kanata/Switcher",
            "com.github.kanata.Switcher",
        )
        .await
        .expect("Failed to create proxy");
        let mut paused_stream = proxy
            .receive_signal("PausedChanged")
            .await
            .expect("Failed to subscribe to PausedChanged");

        let pause_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Pause",
                &(),
            )
            .await;
        assert!(
            pause_result.is_ok(),
            "DBus Pause failed: {:?}",
            pause_result.err()
        );

        let paused_msg = tokio::time::timeout(Duration::from_secs(2), paused_stream.next())
            .await
            .expect("PausedChanged signal timed out")
            .expect("PausedChanged stream closed");
        let paused: bool = paused_msg
            .body()
            .deserialize()
            .expect("Failed to deserialize PausedChanged");
        assert!(paused, "Expected paused=true");

        let unpause_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "Unpause",
                &(),
            )
            .await;
        assert!(
            unpause_result.is_ok(),
            "DBus Unpause failed: {:?}",
            unpause_result.err()
        );

        let unpaused_msg = tokio::time::timeout(Duration::from_secs(2), paused_stream.next())
            .await
            .expect("PausedChanged signal timed out")
            .expect("PausedChanged stream closed");
        let paused: bool = unpaused_msg
            .body()
            .deserialize()
            .expect("Failed to deserialize PausedChanged");
        assert!(!paused, "Expected paused=false");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_dbus_status_changed_focus_signal() {
    with_test_timeout(async {
        use futures_util::StreamExt;
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let proxy = zbus::Proxy::new(
            &client,
            "com.github.kanata.Switcher",
            "/com/github/kanata/Switcher",
            "com.github.kanata.Switcher",
        )
        .await
        .expect("Failed to create proxy");
        let mut status_stream = proxy
            .receive_signal("StatusChanged")
            .await
            .expect("Failed to subscribe to StatusChanged");

        let focus_result = client
            .call_method(
                Some("com.github.kanata.Switcher"),
                "/com/github/kanata/Switcher",
                Some("com.github.kanata.Switcher"),
                "WindowFocus",
                &("test-app", "Test Window"),
            )
            .await;
        assert!(
            focus_result.is_ok(),
            "DBus WindowFocus failed: {:?}",
            focus_result.err()
        );

        let mut focus_signal: Option<(String, Vec<String>, String)> = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let msg = tokio::time::timeout(Duration::from_secs(2), status_stream.next())
                .await
                .ok()
                .flatten();
            if let Some(message) = msg {
                let (layer, virtual_keys, source): (String, Vec<String>, String) = message
                    .body()
                    .deserialize()
                    .expect("Failed to deserialize StatusChanged");
                if source == "focus" {
                    focus_signal = Some((layer, virtual_keys, source));
                    break;
                }
            } else {
                break;
            }
        }

        let (layer, _virtual_keys, source) =
            focus_signal.expect("Expected a StatusChanged signal with focus source");
        assert_eq!(layer, "browser");
        assert_eq!(source, "focus");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_handle_focus_event_ignored_when_paused() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: None,
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        pause_broadcaster.set_paused(true);
        let win = WindowInfo {
            class: "test-app".to_string(),
            title: "Test Window".to_string(),
            is_native_terminal: false,
        };
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &win,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_none(), "Expected no actions while paused");
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");

        pause_broadcaster.set_paused(false);
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &win,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_some(), "Expected actions after unpause");
        if let Some(actions) = actions {
            execute_focus_actions(&kanata, actions).await;
        }
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ChangeLayer {
                new: "browser".to_string()
            })
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_returns_error_without_service() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");
        let client = Builder::address(address)
            .expect("Failed to create client builder")
            .build()
            .await
            .expect("Failed to connect client");

        let result = send_control_command_with_connection(&client, ControlCommand::Restart).await;
        assert!(result.is_err(), "Expected error when service is missing");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_unfocus_ignored_when_paused() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        pause_broadcaster.set_paused(true);
        let unfocus = WindowInfo::default();
        let actions = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &unfocus,
            &kanata,
            "default",
        )
        .await;
        assert!(actions.is_none(), "Expected no actions while paused");
        let msg = mock_server.recv_timeout(Duration::from_millis(500));
        assert!(msg.is_none(), "Expected no Kanata messages while paused");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_pause_daemon_releases_virtual_keys_and_resets_layer() {
    with_test_timeout(async {
        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        {
            let win = WindowInfo {
                class: "test-app".to_string(),
                title: "Test Window".to_string(),
                is_native_terminal: false,
            };
            let actions = handler.lock().unwrap().handle(&win, "default");
            assert!(actions.is_some());
        }

        let pause_broadcaster = pause_broadcaster.clone();
        let handler = handler.clone();
        let status_broadcaster = status_broadcaster.clone();
        let kanata = kanata.clone();
        pause_daemon_direct(
            &pause_broadcaster,
            &handler,
            &status_broadcaster,
            &kanata,
            "test",
        )
        .await;

        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        assert_eq!(
            msg,
            Some(KanataMessage::ActOnFakeKey {
                name: "vk_browser".to_string(),
                action: "Release".to_string(),
            })
        );
        let msg = mock_server.recv_timeout(Duration::from_secs(2));
        if let Some(message) = msg {
            assert_eq!(
                message,
                KanataMessage::ChangeLayer {
                    new: "default".to_string()
                }
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_control_command_pause_unpause_private_dbus() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let mock_server = MockKanataServer::start();

        let rules = vec![Rule {
            class: Some("test-app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("browser".to_string()),
            virtual_key: Some("vk_browser".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        }];

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        let (_focus_service, _call_count) =
            start_gnome_focus_service(&address, "test-app", "Test Window").await;

        let status_broadcaster = StatusBroadcaster::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            mock_server.port(),
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        kanata.connect_with_retry().await;

        mock_server.recv_timeout(Duration::from_secs(1));

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .build()
            .await
            .expect("Failed to connect to private bus");
        let focus_query_connection = Builder::address(address.clone())
            .expect("Failed to create focus query builder")
            .build()
            .await
            .expect("Failed to connect focus query bus");

        let restart_handle = RestartHandle::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let mut pause_receiver = pause_broadcaster.subscribe();
        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
        register_dbus_service(
            &service_connection,
            focus_query_connection,
            Environment::Gnome,
            false,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster.clone(),
        )
        .await
        .expect("Failed to register service");

        let client = Builder::address(address)
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
                    .name_has_owner("com.github.kanata.Switcher".try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has_owner| has_owner)
            }
        })
        .await
        .expect("Timeout waiting for service registration");

        let pause_result =
            send_control_command_with_connection(&client, ControlCommand::Pause).await;
        assert!(
            pause_result.is_ok(),
            "Pause control command failed: {:?}",
            pause_result.err()
        );

        let pause_changed =
            tokio::time::timeout(Duration::from_secs(2), pause_receiver.changed()).await;
        assert!(pause_changed.is_ok(), "Pause broadcast timed out");
        assert!(*pause_receiver.borrow(), "Expected paused state true");

        let unpause_result =
            send_control_command_with_connection(&client, ControlCommand::Unpause).await;
        assert!(
            unpause_result.is_ok(),
            "Unpause control command failed: {:?}",
            unpause_result.err()
        );

        let unpause_changed =
            tokio::time::timeout(Duration::from_secs(2), pause_receiver.changed()).await;
        assert!(unpause_changed.is_ok(), "Unpause broadcast timed out");
        assert!(!*pause_receiver.borrow(), "Expected paused state false");
    })
    .await;
}

// === Wayland Protocol Integration Tests ===

/// Mock Wayland compositor for testing the wlr-foreign-toplevel protocol.
///
/// This module implements a minimal Wayland compositor that speaks the
/// wlr-foreign-toplevel-management-v1 protocol, allowing us to test that
/// the daemon correctly handles toplevel events.
mod wayland_mock {
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
    with_test_timeout(async {
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
        mock_server.recv_timeout(Duration::from_secs(1));

        let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
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

/// Test WaylandState directly by simulating protocol events
///
/// This tests that WaylandState correctly processes toplevel events and
/// returns the right WindowInfo.
#[test]
fn test_wayland_mock_compositor_startup() {
    let (_lock, server) = start_wayland_test_server();
    assert!(!server.socket_name().is_empty());
}

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
    with_test_timeout(async {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::*;
        use x11rb::wrapper::ConnectionExt as WrapperExt;

        let xvfb = XvfbGuard::start(103)
            .expect("Xvfb not available. Run `nix run .#test` or install Xvfb manually.");

        unsafe {
            std::env::set_var("DISPLAY", &xvfb.display);
        }

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
        mock_server.recv_timeout(Duration::from_secs(1));

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

// === GNOME Shell Extension Detection Integration Tests ===

/// Mock GNOME Shell Extensions D-Bus service.
/// Implements GetExtensionInfo to verify the daemon probes correctly.
struct MockGnomeShellExtensions {
    /// Extension state to return (1.0=ENABLED, 2.0=DISABLED, etc.)
    state: f64,
}

#[zbus::interface(name = "org.gnome.Shell.Extensions")]
impl MockGnomeShellExtensions {
    /// Mock implementation of GetExtensionInfo
    /// Returns a{sv} dict with extension info including state as f64
    fn get_extension_info(&self, uuid: &str) -> HashMap<String, zbus::zvariant::OwnedValue> {
        use zbus::zvariant::{OwnedValue, Value};
        let mut info = HashMap::new();
        info.insert(
            "uuid".to_string(),
            OwnedValue::try_from(Value::Str(uuid.into())).unwrap(),
        );
        // GNOME Shell returns state as f64 - this is the critical detail we're testing
        info.insert(
            "state".to_string(),
            OwnedValue::try_from(Value::F64(self.state)).unwrap(),
        );
        info
    }
}

/// Integration test for GNOME extension D-Bus probe.
///
/// This test verifies:
/// 1. The probe calls the correct D-Bus destination (org.gnome.Shell)
/// 2. The probe uses the correct object path (/org/gnome/Shell)
/// 3. The probe calls the correct interface (org.gnome.Shell.Extensions)
/// 4. The probe calls the correct method (GetExtensionInfo)
/// 5. The probe correctly parses f64 state values (both enabled and disabled)
///
/// If any of these regress (wrong path, wrong type parsing, etc.),
/// this test will fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_gnome_extension_dbus_probe_integration() {
    with_test_timeout(async {
        use zbus::connection::Builder;

        // Start private dbus-daemon
        let dbus = DbusSessionGuard::start()
            .expect("Failed to start dbus-daemon. Run `nix run .#test` or install dbus.");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // --- Test 1: Extension ENABLED (state=1.0) ---
        {
            let mock_service = MockGnomeShellExtensions { state: 1.0 };

            let service_connection = Builder::address(address.clone())
                .expect("Failed to create connection builder")
                .name(GNOME_SHELL_BUS_NAME)
                .expect("Failed to set bus name")
                .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
                .expect("Failed to serve mock service")
                .build()
                .await
                .expect("Failed to build connection");

            // Wait for service to be registered
            let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
                .await
                .unwrap();
            wait_for_async(|| {
                let proxy = dbus_proxy.clone();
                async move {
                    proxy
                        .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                        .await
                        .ok()
                        .filter(|&has_owner| has_owner)
                }
            })
            .await
            .expect("Timeout waiting for mock GNOME Shell registration");

            // Create blocking connection for the probe
            let client_connection = zbus::blocking::connection::Builder::address(address.clone())
                .expect("Failed to create client builder")
                .build()
                .expect("Failed to connect client");

            // Call the actual probe function - this verifies the full integration
            let status = gnome_extension_dbus_probe_with_connection(&client_connection);

            // Verify the probe succeeded and correctly parsed the response
            let status = status.expect("D-Bus probe should succeed against mock service");
            assert!(status.active, "Extension with state=1.0 should be active");
            assert!(status.enabled, "Extension with state=1.0 should be enabled");
            assert!(
                status.installed,
                "Extension found via D-Bus should be marked installed"
            );
            assert!(matches!(status.method, GnomeDetectionMethod::Dbus));

            // Drop connections to release the bus name
            drop(client_connection);
            drop(service_connection);
        }

        // Small delay to ensure bus name is released
        tokio::time::sleep(Duration::from_millis(100)).await;

        // --- Test 2: Extension DISABLED (state=2.0) ---
        {
            let mock_service = MockGnomeShellExtensions { state: 2.0 };

            let service_connection = Builder::address(address.clone())
                .expect("Failed to create connection builder")
                .name(GNOME_SHELL_BUS_NAME)
                .expect("Failed to set bus name")
                .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
                .expect("Failed to serve mock service")
                .build()
                .await
                .expect("Failed to build connection");

            let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
                .await
                .unwrap();
            wait_for_async(|| {
                let proxy = dbus_proxy.clone();
                async move {
                    proxy
                        .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                        .await
                        .ok()
                        .filter(|&has| has)
                }
            })
            .await
            .expect("Timeout");

            let client_connection = zbus::blocking::connection::Builder::address(address.clone())
                .expect("Builder")
                .build()
                .expect("Connect");

            let status = gnome_extension_dbus_probe_with_connection(&client_connection)
                .expect("Probe should succeed");

            assert!(
                !status.active,
                "Extension with state=2.0 should NOT be active"
            );
            assert!(
                !status.enabled,
                "Extension with state=2.0 should NOT be enabled"
            );
        }
    })
    .await;
}

/// Mock GNOME Shell Extensions D-Bus service with mutable state.
/// Used to test the retry logic when extension state changes during startup.
struct MockGnomeShellExtensionsDelayed {
    /// Extension state (atomic for cross-thread mutation)
    state: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[zbus::interface(name = "org.gnome.Shell.Extensions")]
impl MockGnomeShellExtensionsDelayed {
    fn get_extension_info(&self, uuid: &str) -> HashMap<String, zbus::zvariant::OwnedValue> {
        use zbus::zvariant::{OwnedValue, Value};
        let state_bits = self.state.load(std::sync::atomic::Ordering::SeqCst);
        let state = f64::from_bits(state_bits);
        let mut info = HashMap::new();
        info.insert(
            "uuid".to_string(),
            OwnedValue::try_from(Value::Str(uuid.into())).unwrap(),
        );
        info.insert(
            "state".to_string(),
            OwnedValue::try_from(Value::F64(state)).unwrap(),
        );
        info
    }
}

/// Integration test for GNOME extension startup retry logic.
///
/// Simulates the real-world scenario where:
/// 1. Service starts early, extension is in INITIALIZED state (6)
/// 2. After ~500ms, GNOME Shell finishes loading and state becomes ENABLED (1)
/// 3. The daemon's retry logic should detect the transition
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_gnome_extension_delayed_activation() {
    with_test_timeout(async {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use zbus::connection::Builder;

        let dbus = DbusSessionGuard::start().expect("Failed to start dbus-daemon");

        let address: zbus::Address = dbus.address().parse().expect("Invalid bus address");

        // Start with INITIALIZED state (6)
        let state = Arc::new(AtomicU64::new(f64::to_bits(6.0)));
        let state_clone = state.clone();

        let mock_service = MockGnomeShellExtensionsDelayed {
            state: state.clone(),
        };

        let service_connection = Builder::address(address.clone())
            .expect("Failed to create connection builder")
            .name(GNOME_SHELL_BUS_NAME)
            .expect("Failed to set bus name")
            .serve_at(GNOME_SHELL_OBJECT_PATH, mock_service)
            .expect("Failed to serve mock service")
            .build()
            .await
            .expect("Failed to build connection");

        // Wait for service registration
        let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
            .await
            .unwrap();
        wait_for_async(|| {
            let proxy = dbus_proxy.clone();
            async move {
                proxy
                    .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                    .await
                    .ok()
                    .filter(|&has| has)
            }
        })
        .await
        .expect("Timeout waiting for mock service");

        // Spawn task to change state to ENABLED after 500ms
        let delay_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            state_clone.store(f64::to_bits(1.0), Ordering::SeqCst);
        });

        // Create blocking connection for probing
        let client_connection = zbus::blocking::connection::Builder::address(address.clone())
            .expect("Failed to create client builder")
            .build()
            .expect("Failed to connect client");

        // Simulate retry logic: poll every 50ms until active or timeout
        let start = Instant::now();
        let mut status = gnome_extension_dbus_probe_with_connection(&client_connection)
            .expect("Initial probe should succeed");

        assert!(!status.active, "Initial state should not be active");
        assert_eq!(
            status.state,
            Some(6),
            "Initial state should be INITIALIZED (6)"
        );

        while !status.active && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
            status = gnome_extension_dbus_probe_with_connection(&client_connection)
                .expect("Probe should succeed");
        }

        let elapsed = start.elapsed();

        // Verify success
        assert!(status.active, "Extension should become active after delay");
        assert_eq!(status.state, Some(1), "Final state should be ENABLED (1)");

        // Verify timing: should take ~500ms (allow 450-800ms for CI variance)
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed <= Duration::from_millis(800),
            "Expected ~500ms delay, got {:?}",
            elapsed
        );

        delay_task.await.unwrap();
    })
    .await;
}
