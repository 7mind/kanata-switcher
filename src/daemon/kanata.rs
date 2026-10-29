use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::TcpStream as TokioTcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::Mutex as TokioMutex;
use serde::{Deserialize, Serialize};

use crate::broadcasters::{LayerSource, StatusBroadcaster};

// === Kanata Client ===

#[derive(Serialize)]
pub(crate) struct ChangeLayerMsg {
    #[serde(rename = "ChangeLayer")]
    change_layer: ChangeLayerPayload,
}

#[derive(Serialize)]
pub(crate) struct ChangeLayerPayload {
    new: String,
}

#[derive(Deserialize)]
pub(crate) struct LayerChangeMsg {
    #[serde(rename = "LayerChange")]
    layer_change: Option<LayerChangePayload>,
}

#[derive(Deserialize)]
pub(crate) struct LayerChangePayload {
    new: String,
}

#[derive(Serialize)]
pub(crate) struct RequestLayerNamesMsg {
    #[serde(rename = "RequestLayerNames")]
    request_layer_names: RequestLayerNamesPayload,
}

#[derive(Serialize)]
pub(crate) struct RequestLayerNamesPayload {}

#[derive(Serialize)]
pub(crate) struct ActOnFakeKeyMsg {
    #[serde(rename = "ActOnFakeKey")]
    act_on_fake_key: ActOnFakeKeyPayload,
}

#[derive(Serialize)]
pub(crate) struct ActOnFakeKeyPayload {
    name: String,
    action: String,
}

#[derive(Deserialize)]
pub(crate) struct LayerNamesMsg {
    #[serde(rename = "LayerNames")]
    layer_names: Option<LayerNamesPayload>,
}

#[derive(Deserialize)]
pub(crate) struct LayerNamesPayload {
    names: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct RequestFakeKeyNamesMsg {
    #[serde(rename = "RequestFakeKeyNames")]
    request_fake_key_names: RequestFakeKeyNamesPayload,
}

#[derive(Serialize)]
pub(crate) struct RequestFakeKeyNamesPayload {}

#[derive(Deserialize)]
pub(crate) struct FakeKeyNamesMsg {
    #[serde(rename = "FakeKeyNames")]
    fake_key_names: Option<FakeKeyNamesPayload>,
}

#[derive(Deserialize)]
pub(crate) struct FakeKeyNamesPayload {
    names: Vec<String>,
}

pub(crate) struct KanataClientInner {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) writer: Option<OwnedWriteHalf>,
    pub(crate) reader_handle: Option<tokio::task::JoinHandle<()>>,
    pub(crate) current_layer: Option<String>,
    pub(crate) auto_default_layer: Option<String>,
    pub(crate) config_default_layer: Option<String>,
    pub(crate) pending_layer: Option<String>,
    pub(crate) known_layers: Vec<String>,
    /// Known virtual keys from kanata. None = older kanata (validation disabled),
    /// Some(vec) = validate against this list (even if empty).
    pub(crate) known_virtual_keys: Option<Vec<String>>,
    /// True if kanata doesn't support RequestFakeKeyNames (drops connection on unknown command).
    pub(crate) legacy_kanata: bool,
    pub(crate) connected: bool,
    pub(crate) paused: bool,
    pub(crate) quiet: bool,
    pub(crate) status_broadcaster: StatusBroadcaster,
}

#[derive(Clone)]
pub struct KanataClient {
    pub(crate) inner: Arc<TokioMutex<KanataClientInner>>,
}

impl std::fmt::Debug for KanataClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KanataClient").finish()
    }
}

impl KanataClient {
    pub(crate) fn new(
        host: &str,
        port: u16,
        config_default_layer: Option<String>,
        quiet: bool,
        status_broadcaster: StatusBroadcaster,
    ) -> Self {
        if let Some(ref layer) = config_default_layer {
            println!(
                "[Kanata] Using config-specified default layer: \"{}\"",
                layer
            );
        }
        Self {
            inner: Arc::new(TokioMutex::new(KanataClientInner {
                host: host.to_string(),
                port,
                writer: None,
                reader_handle: None,
                current_layer: None,
                auto_default_layer: None,
                config_default_layer,
                pending_layer: None,
                known_layers: Vec::new(),
                known_virtual_keys: None,
                legacy_kanata: false,
                connected: false,
                paused: false,
                quiet,
                status_broadcaster,
            })),
        }
    }

    fn resolve_layer_name_from_inner(
        inner: &KanataClientInner,
        layer_name: &str,
        warn_unknown: bool,
    ) -> Option<String> {
        if !inner.known_layers.is_empty()
            && !inner.known_layers.iter().any(|layer| layer == layer_name)
        {
            if warn_unknown && !inner.quiet {
                eprintln!(
                    "[Kanata] Warning: Unknown layer \"{}\", switching to default instead",
                    layer_name
                );
            }
            return inner
                .config_default_layer
                .clone()
                .or_else(|| inner.auto_default_layer.clone());
        }
        Some(layer_name.to_string())
    }

    pub(crate) async fn resolve_layer_name(&self, layer_name: &str, warn_unknown: bool) -> Option<String> {
        let inner = self.inner.lock().await;
        Self::resolve_layer_name_from_inner(&inner, layer_name, warn_unknown)
    }

    pub async fn connect_with_retry(&self) {
        let delays = [0, 1000, 2000, 5000];
        let mut attempt = 0;

        loop {
            let delay = delays[attempt.min(delays.len() - 1)];
            if delay > 0 {
                println!("[Kanata] Retrying connection in {}s...", delay / 1000);
                tokio::time::sleep(Duration::from_millis(delay as u64)).await;
            }

            match self.try_connect().await {
                Ok(_) => return,
                Err(e) => {
                    let inner = self.inner.lock().await;
                    eprintln!(
                        "[Kanata] Cannot connect to {}:{}: {}",
                        inner.host, inner.port, e
                    );
                    drop(inner);
                    attempt += 1;
                }
            }
        }
    }

    async fn try_connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (host, port) = {
            let inner = self.inner.lock().await;
            (inner.host.clone(), inner.port)
        };

        let addr = format!("{}:{}", host, port);
        let stream = TokioTcpStream::connect(&addr).await?;
        println!("[Kanata] Connected to {}", addr);

        let (reader, mut writer) = stream.into_split();
        let mut reader = TokioBufReader::new(reader);

        // Read initial LayerChange message
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        let mut current_layer = None;
        if let Ok(msg) = serde_json::from_str::<LayerChangeMsg>(&line)
            && let Some(lc) = msg.layer_change
        {
            println!("[Kanata] Current layer: \"{}\"", lc.new);
            current_layer = Some(lc.new);
        }

        // Request layer names
        let request = RequestLayerNamesMsg {
            request_layer_names: RequestLayerNamesPayload {},
        };
        let request_json = serde_json::to_string(&request).unwrap() + "\n";
        writer.write_all(request_json.as_bytes()).await?;

        // Read LayerNames response
        line.clear();
        reader.read_line(&mut line).await?;

        let mut known_layers = Vec::new();
        // Auto-detect default layer from the first layer in the list (layers are in definition order)
        let mut auto_default_layer = None;
        if let Ok(msg) = serde_json::from_str::<LayerNamesMsg>(&line)
            && let Some(ln) = msg.layer_names
        {
            println!("[Kanata] Available layers: {:?}", ln.names);
            auto_default_layer = ln.names.first().cloned();
            known_layers = ln.names;
        }

        // Request virtual key names (skip if we know this is older kanata)
        let legacy_kanata = {
            let inner = self.inner.lock().await;
            inner.legacy_kanata
        };

        let known_virtual_keys = if legacy_kanata {
            None
        } else {
            let request = RequestFakeKeyNamesMsg {
                request_fake_key_names: RequestFakeKeyNamesPayload {},
            };
            let request_json = serde_json::to_string(&request).unwrap() + "\n";
            writer.write_all(request_json.as_bytes()).await?;

            // Read FakeKeyNames response (or error from older kanata)
            line.clear();
            let read_result = reader.read_line(&mut line).await;

            // If connection was dropped or reset, this is older kanata
            match read_result {
                Err(_) | Ok(0) => {
                    let mut inner = self.inner.lock().await;
                    inner.legacy_kanata = true;
                    return Err("Older kanata detected (no RequestFakeKeyNames support)".into());
                }
                Ok(_) => {}
            }

            // Try to parse as FakeKeyNames response
            if let Ok(msg) = serde_json::from_str::<FakeKeyNamesMsg>(&line) {
                if let Some(fk) = msg.fake_key_names {
                    if !fk.names.is_empty() {
                        println!("[Kanata] Available virtual keys: {:?}", fk.names);
                    }
                    Some(fk.names)
                } else {
                    // No FakeKeyNames in response - older kanata sent error or unexpected response
                    let mut inner = self.inner.lock().await;
                    inner.legacy_kanata = true;
                    return Err("Older kanata detected (no RequestFakeKeyNames support)".into());
                }
            } else {
                // Parsing failed - older kanata sent error response
                let mut inner = self.inner.lock().await;
                inner.legacy_kanata = true;
                return Err("Older kanata detected (no RequestFakeKeyNames support)".into());
            }
        };

        {
            let mut inner = self.inner.lock().await;
            inner.connected = true;
            inner.writer = Some(writer);
            inner.current_layer = current_layer;
            inner.known_layers = known_layers;
            inner.known_virtual_keys = known_virtual_keys;
            if let Some(ref layer) = auto_default_layer {
                if inner.config_default_layer.is_none() {
                    println!("[Kanata] Using auto-detected default layer: \"{}\"", layer);
                }
                inner.auto_default_layer = auto_default_layer;
            }
            if let Some(ref layer) = inner.current_layer {
                inner
                    .status_broadcaster
                    .update_layer(layer.clone(), LayerSource::External);
            }
        }

        let reader_handle = self.clone().spawn_reader(reader);
        let mut inner = self.inner.lock().await;
        inner.reader_handle = Some(reader_handle);
        Ok(())
    }

    fn spawn_reader(
        self,
        mut reader: TokioBufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        println!("[Kanata] Disconnected");
                        {
                            let mut inner = self.inner.lock().await;
                            inner.connected = false;
                            inner.writer = None;
                            inner.reader_handle = None;
                            if inner.paused {
                                return;
                            }
                        }
                        self.reconnect_loop().await;
                        return;
                    }
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<LayerChangeMsg>(&line)
                            && let Some(lc) = msg.layer_change
                        {
                            let mut inner = self.inner.lock().await;
                            if inner.paused {
                                continue;
                            }
                            let old_layer = inner.current_layer.clone();
                            inner.current_layer = Some(lc.new.clone());
                            let status_broadcaster = inner.status_broadcaster.clone();
                            let quiet = inner.quiet;
                            if old_layer.as_ref() != Some(&lc.new) {
                                status_broadcaster
                                    .update_layer(lc.new.clone(), LayerSource::External);
                                if !quiet {
                                    println!(
                                        "[Kanata] Layer changed (external): {} -> {}",
                                        old_layer.as_deref().unwrap_or("(none)"),
                                        lc.new
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[Kanata] Connection error: {}", e);
                        {
                            let mut inner = self.inner.lock().await;
                            inner.connected = false;
                            inner.writer = None;
                            inner.reader_handle = None;
                            if inner.paused {
                                return;
                            }
                        }
                        self.reconnect_loop().await;
                        return;
                    }
                }
            }
        })
    }

    async fn reconnect_loop(&self) {
        let delays = [1000, 2000, 5000];
        let mut attempt = 0;

        loop {
            {
                let inner = self.inner.lock().await;
                if inner.connected || inner.paused {
                    return;
                }
            }

            let delay = delays[attempt.min(delays.len() - 1)];
            println!("[Kanata] Reconnecting in {}s...", delay / 1000);
            tokio::time::sleep(Duration::from_millis(delay as u64)).await;

            match self.try_connect().await {
                Ok(_) => {
                    println!("[Kanata] Reconnected");

                    let pending = {
                        let mut inner = self.inner.lock().await;
                        inner.pending_layer.take()
                    };

                    if let Some(pending) = pending {
                        let current = self.inner.lock().await.current_layer.clone();
                        if current.as_ref() != Some(&pending) {
                            let _ = self.change_layer(&pending).await;
                        }
                    }
                    return;
                }
                Err(_) => {
                    attempt += 1;
                }
            }
        }
    }

    pub async fn change_layer(&self, layer_name: &str) -> bool {
        let mut inner = self.inner.lock().await;

        let target_layer = match Self::resolve_layer_name_from_inner(&inner, layer_name, true) {
            Some(layer) => layer,
            None => return false,
        };

        let current = inner.current_layer.clone();
        if current.as_deref() == Some(&target_layer) {
            return false;
        }

        if !inner.connected {
            inner.pending_layer = Some(target_layer.clone());
            println!(
                "[Kanata] Not connected, will switch to \"{}\" on reconnect",
                target_layer
            );
            return false;
        }

        if let Some(ref mut writer) = inner.writer {
            let msg = ChangeLayerMsg {
                change_layer: ChangeLayerPayload {
                    new: target_layer.clone(),
                },
            };
            let json = serde_json::to_string(&msg).unwrap() + "\n";

            if writer.write_all(json.as_bytes()).await.is_ok() {
                if !inner.quiet {
                    println!(
                        "[Kanata] Switching layer (daemon): {} -> {}",
                        current.as_deref().unwrap_or("(none)"),
                        target_layer
                    );
                }
                inner.current_layer = Some(target_layer);
                return true;
            }
        }
        false
    }

    pub async fn act_on_fake_key(&self, name: &str, action: &str) -> bool {
        let mut inner = self.inner.lock().await;

        if !inner.connected {
            if !inner.quiet {
                eprintln!("[Kanata] Not connected, cannot send fake key action");
            }
            return false;
        }

        // Validate virtual key name if we have the list from kanata
        if Self::filter_valid_virtual_keys(&inner.known_virtual_keys, vec![name.to_string()])
            .is_empty()
        {
            if !inner.quiet {
                eprintln!(
                    "[Kanata] Warning: Unknown virtual key \"{}\", skipping action",
                    name
                );
            }
            return false;
        }

        if let Some(ref mut writer) = inner.writer {
            let msg = ActOnFakeKeyMsg {
                act_on_fake_key: ActOnFakeKeyPayload {
                    name: name.to_string(),
                    action: action.to_string(),
                },
            };
            let json = serde_json::to_string(&msg).unwrap() + "\n";

            if writer.write_all(json.as_bytes()).await.is_ok() {
                if !inner.quiet {
                    println!("[Kanata] Fake key: {} {}", action, name);
                }
                return true;
            }
        }
        false
    }

    pub async fn default_layer(&self) -> Option<String> {
        let inner = self.inner.lock().await;
        inner
            .config_default_layer
            .clone()
            .or_else(|| inner.auto_default_layer.clone())
    }

    pub async fn pause_disconnect(&self) {
        let mut inner = self.inner.lock().await;
        inner.paused = true;
        if let Some(handle) = inner.reader_handle.take() {
            handle.abort();
        }
        if let Some(mut writer) = inner.writer.take() {
            let _ = writer.shutdown().await;
        }
        inner.connected = false;
        inner.current_layer = None;
        inner.auto_default_layer = None;
        inner.pending_layer = None;
        inner.known_layers.clear();
        inner.known_virtual_keys = None;
    }

    pub async fn unpause_connect(&self) {
        {
            let mut inner = self.inner.lock().await;
            inner.paused = false;
        }
        self.connect_with_retry().await;
    }

    /// Filter a list of virtual key names, returning only those that are valid.
    /// If validation is disabled (None), all VKs pass through.
    pub(crate) fn filter_valid_virtual_keys(
        known_virtual_keys: &Option<Vec<String>>,
        vks: Vec<String>,
    ) -> Vec<String> {
        match known_virtual_keys {
            None => vks,
            Some(known_vks) => vks
                .into_iter()
                .filter(|vk| known_vks.contains(vk))
                .collect(),
        }
    }

    pub async fn known_virtual_keys(&self) -> Option<Vec<String>> {
        let inner = self.inner.lock().await;
        inner.known_virtual_keys.clone()
    }

    pub fn default_layer_sync(&self) -> String {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let inner = self.inner.lock().await;
                inner
                    .config_default_layer
                    .clone()
                    .or_else(|| inner.auto_default_layer.clone())
                    .unwrap_or_default()
            })
        })
    }

    pub fn switch_to_default_if_connected_sync(&self) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let default_layer = self.default_layer().await;
                let Some(default_layer) = default_layer else {
                    eprintln!("[Shutdown] No default layer known, skipping reset");
                    return;
                };

                if default_layer.is_empty() {
                    eprintln!("[Shutdown] Default layer is empty, skipping reset");
                    return;
                }

                let mut inner = self.inner.lock().await;
                if !inner.connected {
                    eprintln!("[Shutdown] Not connected to kanata, skipping reset");
                    return;
                }

                if inner.current_layer.as_ref() == Some(&default_layer) {
                    println!("[Shutdown] Already on default layer \"{}\"", default_layer);
                    return;
                }

                if let Some(ref mut writer) = inner.writer {
                    let msg = ChangeLayerMsg {
                        change_layer: ChangeLayerPayload {
                            new: default_layer.clone(),
                        },
                    };
                    let json = serde_json::to_string(&msg).unwrap() + "\n";

                    if writer.write_all(json.as_bytes()).await.is_ok() {
                        println!("[Shutdown] Switched to default layer \"{}\"", default_layer);
                    } else {
                        eprintln!("[Shutdown] Failed to send layer change");
                    }
                }
            })
        })
    }
}

// === Shutdown Guard ===

pub(crate) struct ShutdownGuard {
    kanata: KanataClient,
}

impl ShutdownGuard {
    pub(crate) fn new(kanata: KanataClient) -> Self {
        Self { kanata }
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.kanata.switch_to_default_if_connected_sync();
    }
}
