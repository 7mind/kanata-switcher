use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream as TokioTcpStream;
use tokio::sync::Mutex as TokioMutex;
use wayland_client::{
    backend::ObjectId,
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection as WaylandConnection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use x11rb::connection::Connection as X11Connection;
use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt as X11ConnectionExt, EventMask, Window};
use x11rb::protocol::Event as X11Event;
use x11rb::rust_connection::RustConnection;
use zbus::Connection;

// Generated COSMIC protocols
mod cosmic_workspace {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/cosmic-workspace-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/protocols/cosmic-workspace-unstable-v1.xml");
}

mod cosmic_toplevel {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use crate::cosmic_workspace::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/cosmic-toplevel-info-unstable-v1.xml");
    }
    use self::__interfaces::*;
    use crate::cosmic_workspace::*;
    wayland_scanner::generate_client_code!("src/protocols/cosmic-toplevel-info-unstable-v1.xml");
}

use cosmic_toplevel::{
    zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1},
    zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1},
};
use cosmic_workspace::{
    zcosmic_workspace_handle_v1::ZcosmicWorkspaceHandleV1,
    zcosmic_workspace_group_handle_v1::ZcosmicWorkspaceGroupHandleV1,
    zcosmic_workspace_manager_v1::ZcosmicWorkspaceManagerV1,
};

const GNOME_EXTENSION_UUID: &str = "kanata-switcher@7mind.io";

// === CLI ===

#[derive(Parser)]
#[command(name = "kanata-switcher")]
#[command(about = "Switch kanata layers based on focused window")]
struct Args {
    #[arg(short = 'p', long, default_value = "10000")]
    port: u16,

    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,

    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Quiet mode: suppress focus and layer-switch messages
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Auto-install GNOME extension if missing (default behavior)
    #[arg(long)]
    install_gnome_extension: bool,

    /// Do not auto-install GNOME extension
    #[arg(long)]
    no_install_gnome_extension: bool,
}

fn resolve_install_gnome_extension(matches: &ArgMatches) -> bool {
    use clap::parser::ValueSource;

    let install_from_cli =
        matches.value_source("install_gnome_extension") == Some(ValueSource::CommandLine);
    let no_install_from_cli =
        matches.value_source("no_install_gnome_extension") == Some(ValueSource::CommandLine);

    match (install_from_cli, no_install_from_cli) {
        (false, false) => true,
        (true, false) => true,
        (false, true) => false,
        (true, true) => {
            let install_idx = matches.index_of("install_gnome_extension");
            let no_install_idx = matches.index_of("no_install_gnome_extension");
            match (install_idx, no_install_idx) {
                (Some(i), Some(n)) => i > n,
                _ => true,
            }
        }
    }
}

// === Config ===

/// A rule for matching windows and triggering actions.
/// At least one of `layer`, `virtual_key`, or `raw_vk_action` should be specified.
#[derive(Debug, Clone, Deserialize)]
struct Rule {
    class: Option<String>,
    title: Option<String>,
    /// Layer to switch to when rule matches
    layer: Option<String>,
    /// Virtual key to press while window is focused (auto-released on unfocus)
    virtual_key: Option<String>,
    /// Raw virtual key actions to fire on focus (fire-and-forget)
    /// Format: [["vk_name", "Press|Release|Tap|Toggle"], ...]
    raw_vk_action: Option<Vec<(String, String)>>,
    /// Continue matching subsequent rules after this one
    #[serde(default)]
    fallthrough: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ConfigEntry {
    Default { default: String },
    Rule(Rule),
}

#[derive(Debug, Clone)]
struct Config {
    rules: Vec<Rule>,
    default_layer: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WindowInfo {
    class: String,
    title: String,
}

fn load_config(config_path: Option<&Path>) -> Config {
    let path = config_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            let xdg_config = env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| dirs::home_dir().unwrap().join(".config"));
            xdg_config.join("kanata").join("kanata-switcher.json")
        });

    if !path.exists() {
        eprintln!("[Config] Error: Config file not found: {}", path.display());
        eprintln!();
        eprintln!("Example config:");
        eprintln!(r#"[
  {{"default": "base"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#);
        std::process::exit(1);
    }

    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Vec<ConfigEntry>>(&content) {
            Ok(entries) => {
                let mut rules = Vec::new();
                let mut default_layer: Option<String> = None;

                for entry in entries {
                    match entry {
                        ConfigEntry::Default { default } => {
                            if default_layer.is_some() {
                                eprintln!("[Config] Error: multiple 'default' entries found, only one allowed");
                                std::process::exit(1);
                            }
                            default_layer = Some(default);
                        }
                        ConfigEntry::Rule(rule) => {
                            rules.push(rule);
                        }
                    }
                }

                println!("[Config] Loaded {} rules from {}", rules.len(), path.display());

                Config { rules, default_layer }
            }
            Err(e) => {
                eprintln!("[Config] Error: Failed to parse {}: {}", path.display(), e);
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("[Config] Error: Failed to read {}: {}", path.display(), e);
            std::process::exit(1);
        }
    }
}

fn match_pattern(pattern: Option<&str>, value: &str) -> bool {
    match pattern {
        None => true,
        Some("*") => true,
        Some(pat) => match Regex::new(pat) {
            Ok(re) => re.is_match(value),
            Err(_) => value.contains(pat),
        },
    }
}

// === Focus Handler ===

/// Actions to execute on focus change
#[derive(Debug, Default)]
struct FocusActions {
    /// Layer to switch to (if any)
    layer: Option<String>,
    /// Virtual key to release (previous managed VK)
    release_vk: Option<String>,
    /// Virtual key to press (new managed VK)
    press_vk: Option<String>,
    /// Raw VK actions to fire (fire-and-forget)
    raw_vk_actions: Vec<(String, String)>,
}

impl FocusActions {
    fn is_empty(&self) -> bool {
        self.layer.is_none()
            && self.release_vk.is_none()
            && self.press_vk.is_none()
            && self.raw_vk_actions.is_empty()
    }
}

#[derive(Debug)]
struct FocusHandler {
    rules: Vec<Rule>,
    last_class: String,
    last_title: String,
    current_virtual_key: Option<String>,
    quiet: bool,
}

impl FocusHandler {
    fn new(rules: Vec<Rule>, quiet: bool) -> Self {
        Self {
            rules,
            last_class: String::new(),
            last_title: String::new(),
            current_virtual_key: None,
            quiet,
        }
    }

    /// Handle a focus change event. Returns actions to execute.
    fn handle(&mut self, win: &WindowInfo, default_layer: &str) -> Option<FocusActions> {
        if win.class == self.last_class && win.title == self.last_title {
            return None;
        }

        self.last_class = win.class.clone();
        self.last_title = win.title.clone();

        let mut actions = FocusActions::default();

        // Handle unfocused state (no window has focus)
        if win.class.is_empty() && win.title.is_empty() {
            if !self.quiet {
                println!("[Focus] No window focused");
            }
            // Release any active virtual key
            if let Some(ref vk) = self.current_virtual_key {
                actions.release_vk = Some(vk.clone());
                self.current_virtual_key = None;
            }
            // Switch to default layer
            if !default_layer.is_empty() {
                actions.layer = Some(default_layer.to_string());
            }
            return if actions.is_empty() { None } else { Some(actions) };
        }

        if !self.quiet {
            println!("[Focus] class=\"{}\" title=\"{}\"", win.class, win.title);
        }

        // Match rules with fallthrough support
        let mut matched_layer: Option<String> = None;
        let mut matched_virtual_key: Option<String> = None;

        for rule in &self.rules {
            if match_pattern(rule.class.as_deref(), &win.class)
                && match_pattern(rule.title.as_deref(), &win.title)
            {
                // Collect layer (first match wins)
                if matched_layer.is_none() {
                    if let Some(ref layer) = rule.layer {
                        matched_layer = Some(layer.clone());
                    }
                }

                // Collect virtual_key (first match wins)
                if matched_virtual_key.is_none() {
                    if let Some(ref vk) = rule.virtual_key {
                        matched_virtual_key = Some(vk.clone());
                    }
                }

                // Collect raw_vk_actions (all matches)
                if let Some(ref raw_actions) = rule.raw_vk_action {
                    actions.raw_vk_actions.extend(raw_actions.iter().cloned());
                }

                // Stop if no fallthrough
                if !rule.fallthrough {
                    break;
                }
            }
        }

        // Set layer (use default if no match)
        actions.layer = matched_layer.or_else(|| {
            if default_layer.is_empty() {
                None
            } else {
                Some(default_layer.to_string())
            }
        });

        // Handle virtual key transitions
        let new_vk = matched_virtual_key;
        if new_vk != self.current_virtual_key {
            // Release old VK if different
            if let Some(ref old_vk) = self.current_virtual_key {
                actions.release_vk = Some(old_vk.clone());
            }
            // Press new VK if present
            if let Some(ref vk) = new_vk {
                actions.press_vk = Some(vk.clone());
            }
            self.current_virtual_key = new_vk;
        }

        if actions.is_empty() {
            None
        } else {
            Some(actions)
        }
    }
}

/// Execute focus actions (VK releases, VK presses, raw VK actions, layer change)
async fn execute_focus_actions(kanata: &KanataClient, actions: FocusActions) {
    // Release old virtual key first
    if let Some(ref vk) = actions.release_vk {
        kanata.act_on_fake_key(vk, "Release").await;
    }

    // Press new virtual key
    if let Some(ref vk) = actions.press_vk {
        kanata.act_on_fake_key(vk, "Press").await;
    }

    // Fire raw VK actions
    for (name, action) in &actions.raw_vk_actions {
        kanata.act_on_fake_key(name, action).await;
    }

    // Change layer
    if let Some(ref layer) = actions.layer {
        kanata.change_layer(layer).await;
    }
}

// === Kanata Client ===

#[derive(Serialize)]
struct ChangeLayerMsg {
    #[serde(rename = "ChangeLayer")]
    change_layer: ChangeLayerPayload,
}

#[derive(Serialize)]
struct ChangeLayerPayload {
    new: String,
}

#[derive(Deserialize)]
struct LayerChangeMsg {
    #[serde(rename = "LayerChange")]
    layer_change: Option<LayerChangePayload>,
}

#[derive(Deserialize)]
struct LayerChangePayload {
    new: String,
}

#[derive(Serialize)]
struct RequestLayerNamesMsg {
    #[serde(rename = "RequestLayerNames")]
    request_layer_names: RequestLayerNamesPayload,
}

#[derive(Serialize)]
struct RequestLayerNamesPayload {}

#[derive(Serialize)]
struct ActOnFakeKeyMsg {
    #[serde(rename = "ActOnFakeKey")]
    act_on_fake_key: ActOnFakeKeyPayload,
}

#[derive(Serialize)]
struct ActOnFakeKeyPayload {
    name: String,
    action: String,
}

#[derive(Deserialize)]
struct LayerNamesMsg {
    #[serde(rename = "LayerNames")]
    layer_names: Option<LayerNamesPayload>,
}

#[derive(Deserialize)]
struct LayerNamesPayload {
    names: Vec<String>,
}

struct KanataClientInner {
    host: String,
    port: u16,
    writer: Option<OwnedWriteHalf>,
    current_layer: Option<String>,
    auto_default_layer: Option<String>,
    config_default_layer: Option<String>,
    pending_layer: Option<String>,
    known_layers: Vec<String>,
    connected: bool,
    quiet: bool,
}

#[derive(Clone)]
pub struct KanataClient {
    inner: Arc<TokioMutex<KanataClientInner>>,
}

impl std::fmt::Debug for KanataClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KanataClient").finish()
    }
}

impl KanataClient {
    pub fn new(host: &str, port: u16, config_default_layer: Option<String>, quiet: bool) -> Self {
        if let Some(ref layer) = config_default_layer {
            println!("[Kanata] Using config-specified default layer: \"{}\"", layer);
        }
        Self {
            inner: Arc::new(TokioMutex::new(KanataClientInner {
                host: host.to_string(),
                port,
                writer: None,
                current_layer: None,
                auto_default_layer: None,
                config_default_layer,
                pending_layer: None,
                known_layers: Vec::new(),
                connected: false,
                quiet,
            })),
        }
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
        let mut auto_default_layer = None;
        if let Ok(msg) = serde_json::from_str::<LayerChangeMsg>(&line) {
            if let Some(lc) = msg.layer_change {
                println!("[Kanata] Current layer: \"{}\"", lc.new);
                auto_default_layer = Some(lc.new.clone());
                current_layer = Some(lc.new);
            }
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
        if let Ok(msg) = serde_json::from_str::<LayerNamesMsg>(&line) {
            if let Some(ln) = msg.layer_names {
                println!("[Kanata] Available layers: {:?}", ln.names);
                known_layers = ln.names;
            }
        }

        {
            let mut inner = self.inner.lock().await;
            inner.connected = true;
            inner.writer = Some(writer);
            inner.current_layer = current_layer;
            inner.known_layers = known_layers;
            if let Some(ref layer) = auto_default_layer {
                if inner.config_default_layer.is_none() {
                    println!("[Kanata] Using auto-detected default layer: \"{}\"", layer);
                }
                inner.auto_default_layer = auto_default_layer;
            }
        }

        self.clone().spawn_reader(reader);
        Ok(())
    }

    fn spawn_reader(self, mut reader: TokioBufReader<tokio::net::tcp::OwnedReadHalf>) {
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
                        }
                        self.reconnect_loop().await;
                        return;
                    }
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<LayerChangeMsg>(&line) {
                            if let Some(lc) = msg.layer_change {
                                let mut inner = self.inner.lock().await;
                                let old_layer = inner.current_layer.clone();
                                inner.current_layer = Some(lc.new.clone());
                                if old_layer.as_ref() != Some(&lc.new) && !inner.quiet {
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
                        }
                        self.reconnect_loop().await;
                        return;
                    }
                }
            }
        });
    }

    async fn reconnect_loop(&self) {
        let delays = [1000, 2000, 5000];
        let mut attempt = 0;

        loop {
            {
                let inner = self.inner.lock().await;
                if inner.connected {
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

        // Validate layer exists if we have known layers
        let target_layer = if !inner.known_layers.is_empty()
            && !inner.known_layers.iter().any(|l| l == layer_name)
        {
            // Unknown layer - warn and fall back to default
            if !inner.quiet {
                eprintln!(
                    "[Kanata] Warning: Unknown layer \"{}\", switching to default instead",
                    layer_name
                );
            }
            let default = inner
                .config_default_layer
                .clone()
                .or_else(|| inner.auto_default_layer.clone());
            match default {
                Some(d) => d,
                None => return false,
            }
        } else {
            layer_name.to_string()
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
        inner.config_default_layer.clone().or_else(|| inner.auto_default_layer.clone())
    }

    pub fn default_layer_sync(&self) -> String {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let inner = self.inner.lock().await;
                inner.config_default_layer.clone()
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

struct ShutdownGuard {
    kanata: KanataClient,
}

impl ShutdownGuard {
    fn new(kanata: KanataClient) -> Self {
        Self { kanata }
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.kanata.switch_to_default_if_connected_sync();
    }
}

// === Environment Detection ===

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Gnome,
    Kde,
    Wayland,
    X11,
    Unknown,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Gnome => "gnome",
            Environment::Kde => "kde",
            Environment::Wayland => "wayland",
            Environment::X11 => "x11",
            Environment::Unknown => "unknown",
        }
    }
}

fn detect_environment() -> Environment {
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();

    // GNOME - needs special DBus extension
    if desktop.contains("gnome") || env::var("GNOME_SETUP_DISPLAY").is_ok() {
        return Environment::Gnome;
    }

    // KDE - needs KWin script injection
    if env::var("KDE_SESSION_VERSION").is_ok() {
        return Environment::Kde;
    }

    // Wayland compositors (wlr-based or COSMIC) - use toplevel protocol
    if env::var("WAYLAND_DISPLAY").is_ok() {
        return Environment::Wayland;
    }

    // X11 fallback
    if env::var("DISPLAY").is_ok() {
        return Environment::X11;
    }

    Environment::Unknown
}

// === Wayland Toplevel State ===

#[derive(Default)]
struct ToplevelWindow {
    app_id: String,
    title: String,
}

#[derive(Default)]
struct WaylandState {
    windows: HashMap<ObjectId, ToplevelWindow>,
    active_window: Option<ObjectId>,
}

impl WaylandState {
    fn get_active_window(&self) -> WindowInfo {
        self.active_window
            .as_ref()
            .and_then(|id| self.windows.get(id))
            .map(|w| WindowInfo {
                class: w.app_id.clone(),
                title: w.title.clone(),
            })
            .unwrap_or_default()
    }
}

// === WLR Protocol Dispatch ===

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.windows.insert(toplevel.id(), ToplevelWindow::default());
        }
    }

    wayland_client::event_created_child!(WaylandState, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.app_id = app_id;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.title = title;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: handle_state } => {
                let activated = zwlr_foreign_toplevel_handle_v1::State::Activated as u8;
                if handle_state.contains(&activated) {
                    state.active_window = Some(handle.id());
                } else if state.active_window.as_ref() == Some(&handle.id()) {
                    // Window lost activation - clear active_window
                    state.active_window = None;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.windows.remove(&handle.id());
                if state.active_window.as_ref() == Some(&handle.id()) {
                    state.active_window = None;
                }
            }
            _ => {}
        }
    }
}

// === COSMIC Protocol Dispatch ===

impl Dispatch<ZcosmicToplevelInfoV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let zcosmic_toplevel_info_v1::Event::Toplevel { toplevel } = event {
            state.windows.insert(toplevel.id(), ToplevelWindow::default());
        }
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicToplevelInfoV1, [
        zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (ZcosmicToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicToplevelHandleV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        handle: &ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zcosmic_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.app_id = app_id;
                }
            }
            zcosmic_toplevel_handle_v1::Event::Title { title } => {
                if let Some(w) = state.windows.get_mut(&handle.id()) {
                    w.title = title;
                }
            }
            zcosmic_toplevel_handle_v1::Event::State { state: handle_state } => {
                // COSMIC: activated = 2
                let (chunks, _) = handle_state.as_chunks::<4>();
                let activated = chunks
                    .iter()
                    .map(|&chunk| u32::from_ne_bytes(chunk))
                    .any(|s| s == zcosmic_toplevel_handle_v1::State::Activated as u32);
                if activated {
                    state.active_window = Some(handle.id());
                } else if state.active_window.as_ref() == Some(&handle.id()) {
                    // Window lost activation - clear active_window
                    state.active_window = None;
                }
            }
            zcosmic_toplevel_handle_v1::Event::Closed => {
                state.windows.remove(&handle.id());
                if state.active_window.as_ref() == Some(&handle.id()) {
                    state.active_window = None;
                }
            }
            _ => {}
        }
    }
}

// Dispatch for workspace types (we ignore these events but need to handle them)
impl Dispatch<ZcosmicWorkspaceManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceManagerV1,
        _: cosmic_workspace::zcosmic_workspace_manager_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicWorkspaceManagerV1, [
        cosmic_workspace::zcosmic_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ZcosmicWorkspaceGroupHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicWorkspaceGroupHandleV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceGroupHandleV1,
        _: cosmic_workspace::zcosmic_workspace_group_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(WaylandState, ZcosmicWorkspaceGroupHandleV1, [
        cosmic_workspace::zcosmic_workspace_group_handle_v1::EVT_WORKSPACE_OPCODE => (ZcosmicWorkspaceHandleV1, ())
    ]);
}

impl Dispatch<ZcosmicWorkspaceHandleV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZcosmicWorkspaceHandleV1,
        _: cosmic_workspace::zcosmic_workspace_handle_v1::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// Dispatch for wl_output (referenced by toplevel protocol)
impl Dispatch<wayland_client::protocol::wl_output::WlOutput, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_output::WlOutput,
        _: wayland_client::protocol::wl_output::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// === Wayland Backend ===

#[derive(Debug, Clone, Copy)]
enum WaylandProtocol {
    Wlr,
    Cosmic,
}

async fn run_wayland(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = WaylandConnection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<WaylandState>(&connection)?;

    let mut state = WaylandState::default();

    // Try wlr protocol first, fall back to cosmic
    let protocol = if globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&queue.handle(), 1..=3, ())
        .is_ok()
    {
        WaylandProtocol::Wlr
    } else if globals
        .bind::<ZcosmicToplevelInfoV1, _, _>(&queue.handle(), 1..=1, ())
        .is_ok()
    {
        WaylandProtocol::Cosmic
    } else {
        return Err("No supported toplevel protocol (wlr-foreign-toplevel or cosmic-toplevel-info)".into());
    };

    println!("[Wayland] Using {:?} toplevel protocol", protocol);

    // Initial roundtrip to populate state
    queue.roundtrip(&mut state)?;

    println!("[Wayland] Listening for focus events...");

    let mut handler = FocusHandler::new(rules, quiet);

    loop {
        queue.roundtrip(&mut state)?;

        let win = state.get_active_window();
        let default_layer = kanata.default_layer_sync();

        if let Some(actions) = handler.handle(&win, &default_layer) {
            execute_focus_actions(&kanata, actions).await;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// === X11 Backend ===

x11rb::atom_manager! {
    pub X11Atoms: X11AtomsCookie {
        _NET_WM_NAME,
        _NET_ACTIVE_WINDOW,
        UTF8_STRING,
    }
}

struct X11State {
    connection: RustConnection,
    root: Window,
    atoms: X11Atoms,
}

impl X11State {
    fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (connection, screen_num) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen_num].root;
        let atoms = X11Atoms::new(&connection)?.reply()?;

        // Subscribe to PropertyNotify events on root window
        let attrs = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        connection.change_window_attributes(root, &attrs)?;
        connection.flush()?;

        Ok(Self { connection, root, atoms })
    }

    fn get_active_window_id(&self) -> Option<Window> {
        let prop_reply = self.connection
            .get_property(false, self.root, self.atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ == x11rb::NONE || prop_reply.value.len() != 4 {
            return None;
        }

        let arr: [u8; 4] = prop_reply.value.clone().try_into().ok()?;
        let winid = u32::from_le_bytes(arr);

        if winid == 0 {
            None
        } else {
            Some(winid)
        }
    }

    fn get_window_class(&self, window: Window) -> Option<String> {
        let reply = self.connection
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;

        if reply.value.is_empty() {
            return None;
        }

        // WM_CLASS format: "instance\0class\0"
        // We want just the class part (second element)
        let parts: Vec<&[u8]> = reply.value.split(|&b| b == 0).collect();
        if parts.len() >= 2 {
            String::from_utf8(parts[1].to_vec()).ok()
        } else if !parts.is_empty() {
            String::from_utf8(parts[0].to_vec()).ok()
        } else {
            None
        }
    }

    fn get_window_title(&self, window: Window) -> Option<String> {
        // Try _NET_WM_NAME first (UTF-8)
        let prop_reply = self.connection
            .get_property(false, window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ != x11rb::NONE {
            return String::from_utf8(prop_reply.value).ok();
        }

        // Fallback to WM_NAME (Latin-1)
        let prop_reply = self.connection
            .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()?;

        String::from_utf8(prop_reply.value).ok()
    }

    fn get_active_window(&self) -> WindowInfo {
        let Some(window_id) = self.get_active_window_id() else {
            return WindowInfo::default();
        };

        let class = self.get_window_class(window_id).unwrap_or_default();
        let title = self.get_window_title(window_id).unwrap_or_default();

        WindowInfo { class, title }
    }
}

async fn run_x11(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new()?;

    println!("[X11] Connected to display");

    let mut handler = FocusHandler::new(rules, quiet);

    // Process initial state at boot
    let win = state.get_active_window();
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handler.handle(&win, &default_layer) {
        execute_focus_actions(&kanata, actions).await;
    }

    println!("[X11] Listening for focus events...");

    // Event loop - wait for PropertyNotify events on _NET_ACTIVE_WINDOW
    loop {
        // Use poll_for_event in a loop with small sleep to avoid blocking tokio runtime
        let event = tokio::task::block_in_place(|| state.connection.wait_for_event());

        match event {
            Ok(X11Event::PropertyNotify(e)) if e.atom == state.atoms._NET_ACTIVE_WINDOW => {
                let win = state.get_active_window();
                let default_layer = kanata.default_layer_sync();

                if let Some(actions) = handler.handle(&win, &default_layer) {
                    execute_focus_actions(&kanata, actions).await;
                }
            }
            Ok(_) => {
                // Ignore other events
            }
            Err(e) => {
                eprintln!("[X11] Connection error: {}", e);
                return Err(e.into());
            }
        }
    }
}

// === GNOME Extension Management ===

/// Path to GNOME extension source relative to repository root
const GNOME_EXTENSION_SRC_PATH: &str = "src/gnome-extension";

#[cfg(feature = "embed-gnome-extension")]
macro_rules! gnome_ext_file {
    ($file:literal) => {
        concat!("../../", "src/gnome-extension", "/", $file)
    };
}

#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_EXTENSION_JS: &str = include_str!(gnome_ext_file!("extension.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_METADATA_JSON: &str = include_str!(gnome_ext_file!("metadata.json"));

fn get_gnome_extension_fs_path() -> PathBuf {
    let exe_path = env::current_exe().unwrap();
    let exe_dir = exe_path.parent().unwrap();
    exe_dir.join("gnome")
}

fn gnome_extension_fs_exists() -> bool {
    let path = get_gnome_extension_fs_path();
    path.join("extension.js").exists() && path.join("metadata.json").exists()
}

#[cfg(feature = "embed-gnome-extension")]
fn write_embedded_extension_to_dir(dir: &Path) -> std::io::Result<()> {
    fs::write(dir.join("extension.js"), EMBEDDED_EXTENSION_JS)?;
    fs::write(dir.join("metadata.json"), EMBEDDED_METADATA_JSON)?;
    Ok(())
}

struct GnomeExtensionStatus {
    installed: bool,
    enabled: bool,
}

fn gnome_extension_status() -> GnomeExtensionStatus {
    // Check installed via gnome-extensions info (requires XDG_DATA_DIRS)
    let installed = Command::new("gnome-extensions")
        .args(["info", GNOME_EXTENSION_UUID])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // Check enabled via gsettings (more reliable from systemd services)
    let enabled = Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.contains(GNOME_EXTENSION_UUID)
        })
        .unwrap_or(false);

    GnomeExtensionStatus { installed, enabled }
}

fn print_gnome_extension_install_instructions(reason: &str) {
    let fs_path = get_gnome_extension_fs_path();
    let install_steps = if gnome_extension_fs_exists() {
        format!(
            r#"Extension files are available at: {}

  gnome-extensions pack "{}" --force --out-dir=/tmp
  gnome-extensions install "/tmp/{}.shell-extension.zip" --force
  gnome-extensions enable {}"#,
            fs_path.display(),
            fs_path.display(),
            GNOME_EXTENSION_UUID,
            GNOME_EXTENSION_UUID
        )
    } else {
        format!(
            r#"Clone the repository and install:

  git clone https://github.com/7mind/kanata-switcher.git /tmp/kanata-switcher
  gnome-extensions pack /tmp/kanata-switcher/{} --force --out-dir=/tmp
  gnome-extensions install "/tmp/{}.shell-extension.zip" --force
  gnome-extensions enable {}"#,
            GNOME_EXTENSION_SRC_PATH,
            GNOME_EXTENSION_UUID,
            GNOME_EXTENSION_UUID
        )
    };

    eprintln!(
        r#"
[GNOME] Extension not installed.

{}

To install manually:

{}

Then restart GNOME Shell:
  - Press Alt+F2, type "r", press Enter (X11 only)
  - Or log out and log back in (Wayland)
"#,
        reason, install_steps
    );
}

fn pack_and_install_from_dir(src_dir: &Path, tmp_dir: &Path) -> Result<(), String> {
    let zip_name = format!("{}.shell-extension.zip", GNOME_EXTENSION_UUID);

    let pack_result = Command::new("gnome-extensions")
        .args([
            "pack",
            src_dir.to_str().unwrap(),
            "--force",
            &format!("--out-dir={}", tmp_dir.display()),
        ])
        .output();

    if pack_result.is_err() || !pack_result.as_ref().unwrap().status.success() {
        return Err("gnome-extensions pack failed".to_string());
    }

    let zip_path = tmp_dir.join(&zip_name);
    let install_result = Command::new("gnome-extensions")
        .args(["install", zip_path.to_str().unwrap(), "--force"])
        .output();

    if install_result.is_err() || !install_result.as_ref().unwrap().status.success() {
        return Err("gnome-extensions install failed".to_string());
    }

    Ok(())
}

#[allow(unused_variables, unused_assignments)]
fn install_gnome_extension() -> bool {
    let tmp_dir = tempfile::tempdir().unwrap();
    let fs_path = get_gnome_extension_fs_path();
    let mut fs_error: Option<String> = None;

    // Try filesystem first
    if gnome_extension_fs_exists() {
        println!("[GNOME] Installing from filesystem: {}", fs_path.display());
        match pack_and_install_from_dir(&fs_path, tmp_dir.path()) {
            Ok(()) => {
                println!("[GNOME] Extension installed");
                return true;
            }
            Err(e) => {
                eprintln!("[GNOME] Failed to install from filesystem: {}", e);
                fs_error = Some(e);
            }
        }
    } else {
        eprintln!(
            "[GNOME] Extension files not found at filesystem path: {}",
            fs_path.display()
        );
    }

    // Fallback to embedded extension
    #[cfg(feature = "embed-gnome-extension")]
    {
        eprintln!("[GNOME] Falling back to embedded extension...");
        let embedded_dir = tmp_dir.path().join("embedded");
        fs::create_dir_all(&embedded_dir).unwrap();

        if let Err(e) = write_embedded_extension_to_dir(&embedded_dir) {
            eprintln!("[GNOME] Failed to write embedded extension: {}", e);
            print_gnome_extension_install_instructions(
                "Auto-install failed: could not write embedded extension files.",
            );
            return false;
        }

        match pack_and_install_from_dir(&embedded_dir, tmp_dir.path()) {
            Ok(()) => {
                println!("[GNOME] Extension installed (from embedded)");
                return true;
            }
            Err(e) => {
                eprintln!("[GNOME] Failed to install from embedded: {}", e);
                print_gnome_extension_install_instructions(&format!(
                    "Auto-install failed: {}",
                    e
                ));
                return false;
            }
        }
    }

    #[cfg(not(feature = "embed-gnome-extension"))]
    {
        let reason = if let Some(e) = fs_error {
            format!(
                "Found extension files at {}, but installation failed: {}. \
                 Cannot fall back to embedded extension (disabled in this build).",
                fs_path.display(),
                e
            )
        } else {
            "Extension files not found and embedded extension is disabled in this build.".to_string()
        };
        print_gnome_extension_install_instructions(&reason);
        return false;
    }
}

fn enable_gnome_extension() -> bool {
    let result = Command::new("gnome-extensions")
        .args(["enable", GNOME_EXTENSION_UUID])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            println!("[GNOME] Extension enabled");
            true
        }
        _ => {
            eprintln!("[GNOME] Failed to enable extension");
            eprintln!("[GNOME] Try restarting GNOME Shell first:");
            eprintln!("[GNOME]   - Press Alt+F2, type \"r\", press Enter (X11 only)");
            eprintln!("[GNOME]   - Or log out and log back in (Wayland)");
            eprintln!(
                "[GNOME] Then run: gnome-extensions enable {}",
                GNOME_EXTENSION_UUID
            );
            false
        }
    }
}

fn ensure_gnome_extension(auto_install: bool) -> bool {
    let status = gnome_extension_status();

    if !status.installed {
        if !auto_install {
            print_gnome_extension_install_instructions("Auto-install was disabled (--no-install-gnome-extension).");
            std::process::exit(1);
        }

        println!("[GNOME] Extension not installed, installing...");
        if !install_gnome_extension() {
            std::process::exit(1);
        }
    }

    if !status.enabled {
        println!("[GNOME] Extension not enabled, enabling...");
        if !enable_gnome_extension() {
            std::process::exit(1);
        }
        return true;
    }

    !status.installed
}

fn setup_gnome_extension(auto_install: bool) {
    let status = gnome_extension_status();
    println!(
        "[GNOME] Extension status: {}, {}",
        if status.installed { "installed" } else { "not installed" },
        if status.enabled { "enabled" } else { "not enabled" }
    );

    let needs_restart = ensure_gnome_extension(auto_install);

    if needs_restart {
        println!("[GNOME] Extension installed and enabled.");
        println!("[GNOME] Please restart GNOME Shell to activate the extension.");
        println!("[GNOME]   - Press Alt+F2, type \"r\", press Enter (X11 only)");
        println!("[GNOME]   - Or log out and log back in (Wayland)");
    }
}

// === GNOME Backend ===

async fn run_gnome(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;

    // Register DBus service to receive focus events from extension
    #[derive(Debug)]
    struct GnomeSwitcher {
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        runtime_handle: tokio::runtime::Handle,
    }

    #[zbus::interface(name = "com.github.kanata.Switcher")]
    impl GnomeSwitcher {
        async fn window_focus(&self, window_class: &str, window_title: &str) {
            let win = WindowInfo {
                class: window_class.to_string(),
                title: window_title.to_string(),
            };

            let default_layer = self
                .runtime_handle
                .block_on(async { self.kanata.default_layer().await })
                .unwrap_or_default();

            let actions = self.handler.lock().unwrap().handle(&win, &default_layer);

            if let Some(actions) = actions {
                let kanata = self.kanata.clone();
                self.runtime_handle
                    .block_on(async { execute_focus_actions(&kanata, actions).await });
            }
        }
    }

    let switcher = GnomeSwitcher {
        kanata,
        handler: Arc::new(Mutex::new(FocusHandler::new(rules, quiet))),
        runtime_handle: tokio::runtime::Handle::current(),
    };

    connection
        .object_server()
        .at("/com/github/kanata/Switcher", switcher)
        .await?;

    connection
        .request_name("com.github.kanata.Switcher")
        .await?;

    println!("[GNOME] Listening for focus events from extension...");

    // Wait forever - extension will push focus changes to us
    std::future::pending::<()>().await;
    Ok(())
}

// === KDE Backend ===

async fn run_kde(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;

    let is_kde6 = env::var("KDE_SESSION_VERSION")
        .map(|v| v == "6")
        .unwrap_or(false);

    // Register DBus service BEFORE running KWin script
    #[derive(Debug)]
    struct KdeSwitcher {
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        runtime_handle: tokio::runtime::Handle,
    }

    #[zbus::interface(name = "com.github.kanata.Switcher")]
    impl KdeSwitcher {
        async fn window_focus(&self, window_class: &str, window_title: &str) {
            let win = WindowInfo {
                class: window_class.to_string(),
                title: window_title.to_string(),
            };

            let default_layer = self
                .runtime_handle
                .block_on(async { self.kanata.default_layer().await })
                .unwrap_or_default();

            let actions = self.handler.lock().unwrap().handle(&win, &default_layer);

            if let Some(actions) = actions {
                let kanata = self.kanata.clone();
                self.runtime_handle
                    .block_on(async { execute_focus_actions(&kanata, actions).await });
            }
        }
    }

    let switcher = KdeSwitcher {
        kanata,
        handler: Arc::new(Mutex::new(FocusHandler::new(rules, quiet))),
        runtime_handle: tokio::runtime::Handle::current(),
    };

    connection
        .object_server()
        .at("/com/github/kanata/Switcher", switcher)
        .await?;

    connection
        .request_name("com.github.kanata.Switcher")
        .await?;

    // Now inject KWin script (DBus service is ready to receive calls)
    let api = if is_kde6 { "windowActivated" } else { "clientActivated" };
    let active_window = if is_kde6 { "activeWindow" } else { "activeClient" };
    let kwin_script = format!(
        r#"function notifyFocus(client) {{
  callDBus(
    "com.github.kanata.Switcher",
    "/com/github/kanata/Switcher",
    "com.github.kanata.Switcher",
    "WindowFocus",
    client ? (client.resourceClass || "") : "",
    client ? (client.caption || "") : ""
  );
}}
workspace.{api}.connect(notifyFocus);
notifyFocus(workspace.{active});
"#,
        api = api,
        active = active_window
    );

    let uid = unsafe { libc::getuid() };
    let script_path = format!("/tmp/kanata-switcher-kwin-{}.js", uid);
    fs::write(&script_path, &kwin_script)?;

    for _ in 0..5 {
        let result = connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "loadScript",
                &(&script_path,),
            )
            .await;

        if result.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let _ = connection
        .call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "unloadScript",
            &(&script_path,),
        )
        .await;

    let load_result = connection
        .call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(&script_path,),
        )
        .await?;

    let script_num: i32 = load_result.body().deserialize()?;

    let script_obj_path_str = if is_kde6 {
        format!("/Scripting/Script{}", script_num)
    } else {
        format!("/{}", script_num)
    };

    let script_interface = if is_kde6 {
        "org.kde.kwin.Script"
    } else {
        "org.kde.kwin.Scripting"
    };

    let script_obj_path = zbus::zvariant::ObjectPath::try_from(script_obj_path_str.as_str())?;

    connection
        .call_method(
            Some("org.kde.KWin"),
            script_obj_path,
            Some(script_interface),
            "run",
            &(),
        )
        .await?;

    println!("[KDE] KWin script injected, listening for window focus events...");

    std::future::pending::<()>().await;
    Ok(())
}

// === Main ===

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("[Fatal] {}", e);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    let install_gnome_extension = resolve_install_gnome_extension(&matches);

    let env = detect_environment();
    println!("[Init] Detected environment: {}", env.as_str());

    if env == Environment::Gnome {
        setup_gnome_extension(install_gnome_extension);
    }

    let config = load_config(args.config.as_deref());
    if config.rules.is_empty() {
        eprintln!("[Config] Error: No rules found in config file");
        eprintln!();
        eprintln!("Example config (~/.config/kanata/kanata-switcher.json):");
        eprintln!(r#"[
  {{"default": "base"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#);
        std::process::exit(1);
    }

    let kanata = KanataClient::new(&args.host, args.port, config.default_layer, args.quiet);
    kanata.connect_with_retry().await;

    // Create shutdown guard - will switch to default layer when dropped
    let _shutdown_guard = ShutdownGuard::new(kanata.clone());

    // Set up signal handlers
    let kanata_for_signal = kanata.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("failed to install SIGINT handler");
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler");

        tokio::select! {
            _ = sigterm.recv() => {
                eprintln!("[Signal] Received SIGTERM");
            }
            _ = sigint.recv() => {
                eprintln!("[Signal] Received SIGINT");
            }
            _ = sighup.recv() => {
                eprintln!("[Signal] Received SIGHUP");
            }
        }

        // Switch to default layer using existing connection, then exit
        kanata_for_signal.switch_to_default_if_connected_sync();
        std::process::exit(0);
    });

    match env {
        Environment::Gnome => {
            run_gnome(kanata, config.rules, args.quiet).await?;
        }
        Environment::Kde => {
            run_kde(kanata, config.rules, args.quiet).await?;
        }
        Environment::Wayland => {
            run_wayland(kanata, config.rules, args.quiet).await?;
        }
        Environment::X11 => {
            run_x11(kanata, config.rules, args.quiet).await?;
        }
        Environment::Unknown => {
            eprintln!("[Error] Could not detect display environment");
            eprintln!("[Error] Ensure WAYLAND_DISPLAY or DISPLAY is set");
            std::process::exit(1);
        }
    }

    Ok(())
}
