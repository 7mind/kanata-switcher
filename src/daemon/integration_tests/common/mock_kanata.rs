use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Messages that can be sent to the mock Kanata server
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KanataMessage {
    ChangeLayer { new: String },
    ActOnFakeKey { name: String, action: String },
    RequestLayerNames,
    RequestFakeKeyNames,
}

pub(crate) fn wait_for_kanata_message(
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

pub(crate) fn drain_kanata_messages(server: &MockKanataServer, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        if server.recv_timeout(Duration::from_millis(20)).is_none() {
            break;
        }
    }
}

/// Configuration for MockKanataServer
pub(crate) struct MockKanataConfig {
    /// Virtual keys to report. If None, simulate older kanata that doesn't support the command.
    pub(crate) virtual_keys: Option<Vec<String>>,
}

impl Default for MockKanataConfig {
    fn default() -> Self {
        Self {
            virtual_keys: Some(vec![
                "vk_browser".to_string(),
                "vk_terminal".to_string(),
                "vk_vim".to_string(),
            ]),
        }
    }
}

/// A mock Kanata TCP server for testing
pub(crate) struct MockKanataServer {
    port: u16,
    handle: Option<thread::JoinHandle<()>>,
    receiver: mpsc::Receiver<KanataMessage>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl MockKanataServer {
    pub(crate) fn start() -> Self {
        Self::start_with_config(MockKanataConfig::default())
    }

    /// Start a mock server simulating older kanata that doesn't support RequestFakeKeyNames
    pub(crate) fn start_legacy() -> Self {
        Self::start_with_config(MockKanataConfig { virtual_keys: None })
    }

    pub(crate) fn start_with_config(config: MockKanataConfig) -> Self {
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
                // On macOS, accepted sockets inherit O_NONBLOCK from the listener.
                // Explicitly set blocking mode so BufReader reads work correctly.
                stream.set_nonblocking(false).ok();
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
                                } else if value.get("RequestFakeKeyNames").is_some() {
                                    sender.send(KanataMessage::RequestFakeKeyNames).ok();
                                    // Respond based on config
                                    match &config.virtual_keys {
                                        Some(vks) => {
                                            let names_json = serde_json::to_string(vks).unwrap();
                                            let response = format!(
                                                r#"{{"FakeKeyNames":{{"names":{}}}}}"#,
                                                names_json
                                            );
                                            writeln!(stream, "{}", response).ok();
                                        }
                                        None => {
                                            // Simulate older kanata: send error then drop connection
                                            let response = r#"{"status":"Error","msg":"Failed to deserialize command: unknown variant `RequestFakeKeyNames`"}"#;
                                            writeln!(stream, "{}", response).ok();
                                            break;
                                        }
                                    }
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

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Option<KanataMessage> {
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
