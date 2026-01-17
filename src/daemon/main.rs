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
use tokio::sync::{watch, Mutex as TokioMutex};
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
use zbus::object_server::SignalEmitter;
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

    /// Suppress focus messages only
    #[arg(long)]
    quiet_focus: bool,

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

/// Individual action to execute on focus change
#[derive(Debug, Clone, PartialEq, Eq)]
enum FocusAction {
    /// Release a virtual key
    ReleaseVk(String),
    /// Switch to a layer
    ChangeLayer(String),
    /// Press and hold a virtual key (managed - will be released on next focus change)
    PressVk(String),
    /// Raw VK action (name, action: Press/Release/Tap/Toggle)
    RawVkAction(String, String),
}

/// Actions to execute on focus change, in order.
/// With fallthrough, all matching actions are collected and executed sequentially.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct FocusActions {
    /// Ordered list of actions to execute
    actions: Vec<FocusAction>,
    /// The new ordered list of managed VKs after execution (pressed top-to-bottom, released bottom-to-top)
    new_managed_vks: Vec<String>,
}

impl FocusActions {
    fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[derive(Debug)]
struct FocusHandler {
    rules: Vec<Rule>,
    last_class: String,
    last_title: String,
    /// Currently held virtual keys, in order they were pressed (top-to-bottom rule order)
    current_virtual_keys: Vec<String>,
    quiet_focus: bool,
}

impl FocusHandler {
    fn new(rules: Vec<Rule>, quiet_focus: bool) -> Self {
        Self {
            rules,
            last_class: String::new(),
            last_title: String::new(),
            current_virtual_keys: Vec::new(),
            quiet_focus,
        }
    }

    /// Handle a focus change event. Returns actions to execute.
    /// With fallthrough, ALL matching actions are collected and executed in order.
    /// All matched virtual_keys are pressed and held simultaneously.
    fn handle(&mut self, win: &WindowInfo, default_layer: &str) -> Option<FocusActions> {
        if win.class == self.last_class && win.title == self.last_title {
            return None;
        }

        self.last_class = win.class.clone();
        self.last_title = win.title.clone();

        let mut result = FocusActions::default();

        // Handle unfocused state (no window has focus)
        if win.class.is_empty() && win.title.is_empty() {
            if !self.quiet_focus {
                println!("[Focus] No window focused");
            }
            // Release all active virtual keys in reverse order (bottom-to-top)
            for vk in self.current_virtual_keys.iter().rev() {
                result.actions.push(FocusAction::ReleaseVk(vk.clone()));
            }
            // Switch to default layer
            if !default_layer.is_empty() {
                result.actions.push(FocusAction::ChangeLayer(default_layer.to_string()));
            }
            result.new_managed_vks = Vec::new();
            self.current_virtual_keys = Vec::new();
            return if result.is_empty() { None } else { Some(result) };
        }

        if !self.quiet_focus {
            println!("[Focus] class=\"{}\" title=\"{}\"", win.class, win.title);
        }

        // Match rules with fallthrough support
        struct MatchedRule {
            layer: Option<String>,
            virtual_key: Option<String>,
            raw_vk_actions: Vec<(String, String)>,
        }

        let mut matched_rules: Vec<MatchedRule> = Vec::new();

        for rule in &self.rules {
            if match_pattern(rule.class.as_deref(), &win.class)
                && match_pattern(rule.title.as_deref(), &win.title)
            {
                matched_rules.push(MatchedRule {
                    layer: rule.layer.clone(),
                    virtual_key: rule.virtual_key.clone(),
                    raw_vk_actions: rule.raw_vk_action.clone().unwrap_or_default(),
                });

                if !rule.fallthrough {
                    break;
                }
            }
        }

        // Collect all VKs from matched rules in order (for holding)
        let new_vks: Vec<String> = matched_rules
            .iter()
            .filter_map(|r| r.virtual_key.clone())
            .collect();

        // Release VKs that are no longer matched (in reverse order)
        for vk in self.current_virtual_keys.iter().rev() {
            if !new_vks.contains(vk) {
                result.actions.push(FocusAction::ReleaseVk(vk.clone()));
            }
        }

        // If no rules matched, use default layer
        if matched_rules.is_empty() {
            if !default_layer.is_empty() {
                result.actions.push(FocusAction::ChangeLayer(default_layer.to_string()));
            }
            result.new_managed_vks = Vec::new();
        } else {
            // Process matched rules in order, building action list
            for matched in matched_rules {
                // Layer change
                if let Some(layer) = matched.layer {
                    result.actions.push(FocusAction::ChangeLayer(layer));
                }

                // Virtual key: press if not already held
                if let Some(ref vk) = matched.virtual_key {
                    if !self.current_virtual_keys.contains(vk) {
                        result.actions.push(FocusAction::PressVk(vk.clone()));
                    }
                }

                // Raw VK actions
                for (name, action) in matched.raw_vk_actions {
                    result.actions.push(FocusAction::RawVkAction(name, action));
                }
            }
            result.new_managed_vks = new_vks;
        }

        // Update state
        self.current_virtual_keys = result.new_managed_vks.clone();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn current_virtual_keys(&self) -> Vec<String> {
        self.current_virtual_keys.clone()
    }

    fn reset(&mut self) {
        self.last_class.clear();
        self.last_title.clear();
        self.current_virtual_keys.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusSnapshot {
    layer: String,
    virtual_keys: Vec<String>,
    layer_source: LayerSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LayerSource {
    Focus,
    External,
}

impl LayerSource {
    fn as_str(&self) -> &'static str {
        match self {
            LayerSource::Focus => "focus",
            LayerSource::External => "external",
        }
    }
}

#[derive(Clone, Debug)]
struct StatusBroadcaster {
    sender: watch::Sender<StatusSnapshot>,
}

#[derive(Clone, Debug)]
struct RestartHandle {
    sender: watch::Sender<bool>,
}

#[derive(Clone, Debug)]
struct PauseBroadcaster {
    sender: watch::Sender<bool>,
}

impl RestartHandle {
    fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    fn request(&self) {
        self.sender.send_replace(true);
    }
}

impl PauseBroadcaster {
    fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }

    fn is_paused(&self) -> bool {
        *self.sender.borrow()
    }

    fn set_paused(&self, paused: bool) -> bool {
        let current = *self.sender.borrow();
        if current == paused {
            return false;
        }
        self.sender.send_replace(paused);
        true
    }
}

impl StatusBroadcaster {
    fn new() -> Self {
        let initial = StatusSnapshot {
            layer: String::new(),
            virtual_keys: Vec::new(),
            layer_source: LayerSource::External,
        };
        let (sender, _) = watch::channel(initial);
        Self { sender }
    }

    fn subscribe(&self) -> watch::Receiver<StatusSnapshot> {
        self.sender.subscribe()
    }

    fn snapshot(&self) -> StatusSnapshot {
        self.sender.borrow().clone()
    }

    fn update_layer(&self, layer: String, source: LayerSource) {
        self.update(|state| {
            state.layer = layer;
            state.layer_source = source;
        });
    }

    fn update_virtual_keys(&self, virtual_keys: Vec<String>) {
        self.update(|state| {
            state.virtual_keys = virtual_keys;
        });
    }

    fn update_focus_layer(&self, layer: String) {
        let mut next = self.sender.borrow().clone();
        next.layer = layer;
        next.layer_source = LayerSource::Focus;
        self.sender.send_replace(next);
    }

    fn set_paused_status(&self, layer: String) {
        let mut next = self.sender.borrow().clone();
        next.layer = layer;
        next.layer_source = LayerSource::External;
        next.virtual_keys = Vec::new();
        self.sender.send_replace(next);
    }

    fn update<F>(&self, updater: F)
    where
        F: FnOnce(&mut StatusSnapshot),
    {
        let current = self.sender.borrow().clone();
        let mut next = current.clone();
        updater(&mut next);
        if next != current {
            self.sender.send_replace(next);
        }
    }
}

/// Execute focus actions in order
async fn execute_focus_actions(kanata: &KanataClient, actions: FocusActions) {
    for action in actions.actions {
        match action {
            FocusAction::ReleaseVk(vk) => {
                kanata.act_on_fake_key(&vk, "Release").await;
            }
            FocusAction::ChangeLayer(layer) => {
                kanata.change_layer(&layer).await;
            }
            FocusAction::PressVk(vk) => {
                kanata.act_on_fake_key(&vk, "Press").await;
            }
            FocusAction::RawVkAction(name, action) => {
                kanata.act_on_fake_key(&name, &action).await;
            }
        }
    }
}

fn extract_focus_layer(actions: &FocusActions) -> Option<String> {
    actions.actions.iter().fold(None, |last, action| {
        if let FocusAction::ChangeLayer(layer) = action {
            Some(layer.clone())
        } else {
            last
        }
    })
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
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    current_layer: Option<String>,
    auto_default_layer: Option<String>,
    config_default_layer: Option<String>,
    pending_layer: Option<String>,
    known_layers: Vec<String>,
    connected: bool,
    paused: bool,
    quiet: bool,
    status_broadcaster: StatusBroadcaster,
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
    fn new(
        host: &str,
        port: u16,
        config_default_layer: Option<String>,
        quiet: bool,
        status_broadcaster: StatusBroadcaster,
    ) -> Self {
        if let Some(ref layer) = config_default_layer {
            println!("[Kanata] Using config-specified default layer: \"{}\"", layer);
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
                connected: false,
                paused: false,
                quiet,
                status_broadcaster,
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
            if let Some(ref layer) = inner.current_layer {
                inner.status_broadcaster
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
                        if let Ok(msg) = serde_json::from_str::<LayerChangeMsg>(&line) {
                            if let Some(lc) = msg.layer_change {
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
    }

    pub async fn unpause_connect(&self) {
        {
            let mut inner = self.inner.lock().await;
            inner.paused = false;
        }
        self.connect_with_retry().await;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunOutcome {
    Restart,
    Exit,
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
    quiet_focus: bool,
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

    let mut handler = FocusHandler::new(rules, quiet_focus);

    // Process initial state
    let win = state.get_active_window();
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handler.handle(&win, &default_layer) {
        execute_focus_actions(&kanata, actions).await;
    }

    // Event loop - block until events arrive
    loop {
        // blocking_dispatch waits for events instead of polling
        let result = tokio::task::block_in_place(|| queue.blocking_dispatch(&mut state));

        if let Err(e) = result {
            eprintln!("[Wayland] Dispatch error: {}", e);
            return Err(e.into());
        }

        let win = state.get_active_window();
        let default_layer = kanata.default_layer_sync();

        if let Some(actions) = handler.handle(&win, &default_layer) {
            execute_focus_actions(&kanata, actions).await;
        }
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
    quiet_focus: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new()?;

    println!("[X11] Connected to display");

    let mut handler = FocusHandler::new(rules, quiet_focus);

    // Process initial state at boot
    let win = state.get_active_window();
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handler.handle(&win, &default_layer) {
        execute_focus_actions(&kanata, actions).await;
    }

    println!("[X11] Listening for focus events...");

    // Event loop - wait for PropertyNotify events on _NET_ACTIVE_WINDOW
    loop {
        // block_in_place allows blocking I/O in async context
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
const GNOME_EXTENSION_SCHEMA_FILE: &str =
    "schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml";
const GNOME_EXTENSION_SCHEMA_COMPILED: &str = "schemas/gschemas.compiled";

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
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_PREFS_JS: &str = include_str!(gnome_ext_file!("prefs.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_FORMAT_JS: &str = include_str!(gnome_ext_file!("format.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_GSETTINGS_SCHEMA: &str = include_str!(gnome_ext_file!(
    "schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml"
));

fn get_gnome_extension_fs_path() -> PathBuf {
    let exe_path = env::current_exe().unwrap();
    let exe_dir = exe_path.parent().unwrap();
    exe_dir.join("gnome")
}

fn gnome_extension_fs_exists() -> bool {
    let path = get_gnome_extension_fs_path();
    path.join("extension.js").exists()
        && path.join("metadata.json").exists()
        && path.join("prefs.js").exists()
        && path.join("format.js").exists()
        && path.join(GNOME_EXTENSION_SCHEMA_FILE).exists()
        && path.join(GNOME_EXTENSION_SCHEMA_COMPILED).exists()
}

#[cfg(feature = "embed-gnome-extension")]
fn compile_gnome_schemas(dir: &Path) -> std::io::Result<()> {
    let schema_dir = dir.join("schemas");
    let output = Command::new("glib-compile-schemas").arg(&schema_dir).output()?;
    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "glib-compile-schemas failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(())
}

#[cfg(feature = "embed-gnome-extension")]
fn write_embedded_extension_to_dir(dir: &Path) -> std::io::Result<()> {
    fs::write(dir.join("extension.js"), EMBEDDED_EXTENSION_JS)?;
    fs::write(dir.join("metadata.json"), EMBEDDED_METADATA_JSON)?;
    fs::write(dir.join("prefs.js"), EMBEDDED_PREFS_JS)?;
    fs::write(dir.join("format.js"), EMBEDDED_FORMAT_JS)?;
    let schema_dir = dir.join("schemas");
    fs::create_dir_all(&schema_dir)?;
    fs::write(dir.join(GNOME_EXTENSION_SCHEMA_FILE), EMBEDDED_GSETTINGS_SCHEMA)?;
    compile_gnome_schemas(dir)?;
    Ok(())
}

enum GnomeDetectionMethod {
    /// Detected via D-Bus call to org.gnome.Shell.Extensions
    Dbus,
    /// Detected via gnome-extensions CLI and gsettings
    Cli,
}

struct GnomeExtensionStatus {
    installed: bool,
    enabled: bool,
    /// Extension is active in GNOME Shell (verified via D-Bus)
    active: bool,
    /// Raw state from D-Bus (None for CLI detection)
    /// 1=ENABLED, 2=DISABLED, 3=ERROR, 4=OUT_OF_DATE, 5=DOWNLOADING, 6=INITIALIZED
    state: Option<u8>,
    /// How the status was detected
    method: GnomeDetectionMethod,
}

fn gnome_state_name(state: u8) -> &'static str {
    match state {
        1 => "enabled",
        2 => "disabled",
        3 => "error",
        4 => "out_of_date",
        5 => "downloading",
        6 => "initialized",
        _ => "unknown",
    }
}

/// Parse GNOME Shell extension state from D-Bus response.
/// State values: 1.0=ENABLED, 2.0=DISABLED, 3.0=ERROR, 4.0=OUT_OF_DATE, 5.0=DOWNLOADING, 6.0=INITIALIZED
#[cfg_attr(test, allow(dead_code))]
fn parse_gnome_extension_state(body: &HashMap<String, zbus::zvariant::OwnedValue>) -> GnomeExtensionStatus {
    // State is returned as f64 by GNOME Shell D-Bus API
    let state_f64: f64 = body
        .get("state")
        .and_then(|v| v.downcast_ref::<f64>().ok())
        .unwrap_or(0.0);
    let state = state_f64 as u8;

    // State 1 = ENABLED (active)
    let active = state == 1;

    GnomeExtensionStatus {
        installed: true,
        enabled: active,
        active,
        state: Some(state),
        method: GnomeDetectionMethod::Dbus,
    }
}

// D-Bus coordinates for GNOME Shell Extensions interface
const GNOME_SHELL_BUS_NAME: &str = "org.gnome.Shell";
const GNOME_SHELL_OBJECT_PATH: &str = "/org/gnome/Shell";
const GNOME_SHELL_EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";

/// Quick probe: check if extension is active via D-Bus call to GNOME Shell.
/// This bypasses filesystem searches and works reliably from systemd services.
fn gnome_extension_dbus_probe() -> Option<GnomeExtensionStatus> {
    let connection = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[GNOME] D-Bus probe: failed to connect to session bus: {}", e);
            return None;
        }
    };
    gnome_extension_dbus_probe_with_connection(&connection)
}

/// Probe using a specific D-Bus connection (for testing with mock services)
fn gnome_extension_dbus_probe_with_connection(
    connection: &zbus::blocking::Connection,
) -> Option<GnomeExtensionStatus> {
    let reply = match connection.call_method(
        Some(GNOME_SHELL_BUS_NAME),
        GNOME_SHELL_OBJECT_PATH,
        Some(GNOME_SHELL_EXTENSIONS_INTERFACE),
        "GetExtensionInfo",
        &(GNOME_EXTENSION_UUID,),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[GNOME] D-Bus probe: GetExtensionInfo call failed: {}", e);
            return None;
        }
    };

    // Response is a dict (a{sv}) with extension info
    let body: HashMap<String, zbus::zvariant::OwnedValue> = match reply.body().deserialize() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[GNOME] D-Bus probe: failed to deserialize response: {}", e);
            return None;
        }
    };

    Some(parse_gnome_extension_state(&body))
}

fn gnome_extension_status() -> GnomeExtensionStatus {
    // Quick probe: try D-Bus call to GNOME Shell first
    // This is the most reliable method from systemd services
    if let Some(status) = gnome_extension_dbus_probe() {
        return status;
    }

    // Fallback: CLI tools (may fail from systemd if XDG_DATA_DIRS is incomplete)

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

    GnomeExtensionStatus { installed, enabled, active: false, state: None, method: GnomeDetectionMethod::Cli }
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

fn ensure_gnome_extension(status: &GnomeExtensionStatus, auto_install: bool) -> bool {
    // If D-Bus probe confirmed extension is active, we're done
    if status.active {
        return false;
    }

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

fn print_gnome_extension_status(status: &GnomeExtensionStatus) {
    let method_str = match status.method {
        GnomeDetectionMethod::Dbus => "via D-Bus",
        GnomeDetectionMethod::Cli => "via gnome-extensions",
    };

    if status.active {
        println!("[GNOME] Extension status: active ({})", method_str);
    } else {
        let state_info = status
            .state
            .map(|s| format!(", state={}", gnome_state_name(s)))
            .unwrap_or_default();
        println!(
            "[GNOME] Extension status: {}, {} ({}{}){}",
            if status.installed { "installed" } else { "not installed" },
            if status.enabled { "enabled" } else { "not enabled" },
            method_str,
            state_info,
            if !matches!(status.state, Some(2) | Some(4)) { " - waiting for GNOME Shell..." } else { "" }
        );
    }
}

fn setup_gnome_extension(auto_install: bool) {
    // Retry settings for when extension is installed but GNOME Shell is still loading
    const RETRY_INTERVAL_MS: u64 = 50;
    const MAX_WAIT_MS: u64 = 30_000;
    const MAX_RETRIES: u64 = MAX_WAIT_MS / RETRY_INTERVAL_MS;

    let mut status = gnome_extension_status();
    print_gnome_extension_status(&status);

    // Retry on all states except:
    // - DISABLED (2): user explicitly disabled the extension
    // - OUT_OF_DATE (4): extension doesn't support current GNOME Shell version
    let is_transient_state = |s: Option<u8>| !matches!(s, Some(2) | Some(4));

    if status.installed && !status.active && is_transient_state(status.state) {
        let initial_state = status.state;
        let mut elapsed_ms: u64 = 0;
        for attempt in 0..MAX_RETRIES {
            std::thread::sleep(std::time::Duration::from_millis(RETRY_INTERVAL_MS));
            elapsed_ms += RETRY_INTERVAL_MS;
            status = gnome_extension_status();

            if status.active {
                println!("[GNOME] Extension became active after {}ms", elapsed_ms);
                print_gnome_extension_status(&status);
                return;
            }

            if !is_transient_state(status.state) {
                println!(
                    "[GNOME] Extension state changed to {} after {}ms",
                    status.state.map(gnome_state_name).unwrap_or("unknown"),
                    elapsed_ms
                );
                break;
            }

            // Log progress every second
            if (attempt + 1) % 20 == 0 {
                println!(
                    "[GNOME] Still waiting for extension to load (state={})... ({}ms/{}ms)",
                    initial_state.map(gnome_state_name).unwrap_or("unknown"),
                    elapsed_ms,
                    MAX_WAIT_MS
                );
            }
        }

        if !status.active {
            print_gnome_extension_status(&status);
        }
    }

    let needs_restart = ensure_gnome_extension(&status, auto_install);

    if needs_restart {
        println!("[GNOME] Extension installed and enabled.");
        println!("[GNOME] Please restart GNOME Shell to activate the extension.");
        println!("[GNOME]   - Press Alt+F2, type \"r\", press Enter (X11 only)");
        println!("[GNOME]   - Or log out and log back in (Wayland)");
    }
}

// === DBus Backend (shared by GNOME and KDE) ===

#[derive(Debug)]
struct DbusWindowFocusService {
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    runtime_handle: tokio::runtime::Handle,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
}

#[zbus::interface(name = "com.github.kanata.Switcher")]
impl DbusWindowFocusService {
    async fn window_focus(&self, window_class: &str, window_title: &str) {
        if self.pause_broadcaster.is_paused() {
            return;
        }

        let win = WindowInfo {
            class: window_class.to_string(),
            title: window_title.to_string(),
        };

        let default_layer = self
            .runtime_handle
            .block_on(async { self.kanata.default_layer().await })
            .unwrap_or_default();

        let (actions, virtual_keys, focus_layer) = {
            let mut handler = self.handler.lock().unwrap();
            let actions = handler.handle(&win, &default_layer);
            let virtual_keys = handler.current_virtual_keys();
            let focus_layer = actions
                .as_ref()
                .and_then(|focus_actions| extract_focus_layer(focus_actions));
            (actions, virtual_keys, focus_layer)
        };

        self.status_broadcaster.update_virtual_keys(virtual_keys);
        if let Some(layer) = focus_layer {
            self.status_broadcaster.update_focus_layer(layer);
        }

        if let Some(actions) = actions {
            let kanata = self.kanata.clone();
            self.runtime_handle
                .block_on(async { execute_focus_actions(&kanata, actions).await });
        }
    }

    async fn get_status(&self) -> (String, Vec<String>, String) {
        let snapshot = self.status_broadcaster.snapshot();
        (
            snapshot.layer,
            snapshot.virtual_keys,
            snapshot.layer_source.as_str().to_string(),
        )
    }

    async fn get_paused(&self) -> bool {
        self.pause_broadcaster.is_paused()
    }

    #[zbus(signal)]
    async fn status_changed(
        signal_emitter: &SignalEmitter<'_>,
        layer: &str,
        virtual_keys: &[&str],
        source: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn paused_changed(
        signal_emitter: &SignalEmitter<'_>,
        paused: bool,
    ) -> zbus::Result<()>;

    async fn restart(&self) {
        println!("[Restart] Restart requested via DBus");
        self.restart_handle.request();
    }

    async fn pause(&self) {
        if !self.pause_broadcaster.set_paused(true) {
            println!("[Pause] Pause requested via DBus (already paused)");
            return;
        }
        println!("[Pause] Pausing daemon");
        let virtual_keys = {
            let mut handler = self.handler.lock().unwrap();
            let keys = handler.current_virtual_keys();
            handler.reset();
            keys
        };
        let status_broadcaster = self.status_broadcaster.clone();
        let kanata = self.kanata.clone();
        self.runtime_handle.block_on(async move {
            let default_layer = kanata.default_layer().await.unwrap_or_default();

            for vk in virtual_keys.iter().rev() {
                kanata.act_on_fake_key(vk, "Release").await;
            }

            if !default_layer.is_empty() {
                let _ = kanata.change_layer(&default_layer).await;
            }

            status_broadcaster.set_paused_status(default_layer);
            kanata.pause_disconnect().await;
        });
    }

    async fn unpause(&self) {
        if !self.pause_broadcaster.set_paused(false) {
            println!("[Pause] Unpause requested via DBus (already running)");
            return;
        }
        println!("[Pause] Resuming daemon");
        let kanata = self.kanata.clone();
        self.runtime_handle.block_on(async move {
            kanata.unpause_connect().await;
        });
    }
}

async fn register_dbus_service(
    connection: &Connection,
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet_focus: bool,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = DbusWindowFocusService {
        kanata,
        handler: Arc::new(Mutex::new(FocusHandler::new(rules, quiet_focus))),
        runtime_handle: tokio::runtime::Handle::current(),
        status_broadcaster: status_broadcaster.clone(),
        restart_handle,
        pause_broadcaster: pause_broadcaster.clone(),
    };

    connection
        .object_server()
        .at("/com/github/kanata/Switcher", service)
        .await?;

    connection
        .request_name("com.github.kanata.Switcher")
        .await?;

    let mut receiver = status_broadcaster.subscribe();
    let signal_emitter = SignalEmitter::new(connection, "/com/github/kanata/Switcher")?.into_owned();
    let initial_status = status_broadcaster.snapshot();
    let initial_virtual_keys: Vec<&str> = initial_status
        .virtual_keys
        .iter()
        .map(|vk| vk.as_str())
        .collect();
    DbusWindowFocusService::status_changed(
        &signal_emitter,
        &initial_status.layer,
        &initial_virtual_keys,
        initial_status.layer_source.as_str(),
    )
    .await?;
    let signal_emitter_task = signal_emitter.clone();
    tokio::spawn(async move {
        let mut last = receiver.borrow().clone();
        loop {
            if receiver.changed().await.is_err() {
                break;
            }
            let current = receiver.borrow().clone();
            if current != last {
                let virtual_keys: Vec<&str> = current.virtual_keys.iter().map(|vk| vk.as_str()).collect();
                let _ = DbusWindowFocusService::status_changed(
                    &signal_emitter_task,
                    &current.layer,
                    &virtual_keys,
                    current.layer_source.as_str(),
                )
                .await;
                last = current;
            }
        }
    });

    let mut pause_receiver = pause_broadcaster.subscribe();
    let pause_emitter = signal_emitter.clone();
    DbusWindowFocusService::paused_changed(&pause_emitter, pause_broadcaster.is_paused()).await?;
    tokio::spawn(async move {
        let mut last = *pause_receiver.borrow();
        loop {
            if pause_receiver.changed().await.is_err() {
                break;
            }
            let current = *pause_receiver.borrow();
            if current != last {
                let _ = DbusWindowFocusService::paused_changed(&pause_emitter, current).await;
                last = current;
            }
        }
    });

    Ok(())
}

// === GNOME Backend ===

async fn run_gnome(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet_focus: bool,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    register_dbus_service(
        &connection,
        kanata,
        rules,
        quiet_focus,
        status_broadcaster,
        restart_handle.clone(),
        pause_broadcaster,
    )
    .await?;

    println!("[GNOME] Listening for focus events from extension...");
    let mut receiver = restart_handle.subscribe();
    let _ = receiver.changed().await;
    Ok(RunOutcome::Restart)
}

// === KDE Backend ===

async fn run_kde(
    kanata: KanataClient,
    rules: Vec<Rule>,
    quiet_focus: bool,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    register_dbus_service(
        &connection,
        kanata,
        rules,
        quiet_focus,
        status_broadcaster,
        restart_handle.clone(),
        pause_broadcaster,
    )
    .await?;

    let is_kde6 = env::var("KDE_SESSION_VERSION")
        .map(|v| v == "6")
        .unwrap_or(false);

    // Inject KWin script (DBus service is ready to receive calls)
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

    let mut receiver = restart_handle.subscribe();
    let _ = receiver.changed().await;
    Ok(RunOutcome::Restart)
}

// === Main ===

#[tokio::main]
async fn main() {
    loop {
        match run_once().await {
            Ok(RunOutcome::Restart) => {
                println!("[Restart] Restarting daemon");
            }
            Ok(RunOutcome::Exit) => break,
            Err(e) => {
                eprintln!("[Fatal] {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_once() -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
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

    let quiet_focus = args.quiet || args.quiet_focus;
    let status_broadcaster = StatusBroadcaster::new();
    let restart_handle = RestartHandle::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let kanata = KanataClient::new(
        &args.host,
        args.port,
        config.default_layer,
        args.quiet,
        status_broadcaster.clone(),
    );
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
            return run_gnome(
                kanata,
                config.rules,
                quiet_focus,
                status_broadcaster,
                restart_handle,
                pause_broadcaster,
            )
            .await;
        }
        Environment::Kde => {
            return run_kde(
                kanata,
                config.rules,
                quiet_focus,
                status_broadcaster,
                restart_handle,
                pause_broadcaster,
            )
            .await;
        }
        Environment::Wayland => {
            run_wayland(kanata, config.rules, quiet_focus).await?;
        }
        Environment::X11 => {
            run_x11(kanata, config.rules, quiet_focus).await?;
        }
        Environment::Unknown => {
            eprintln!("[Error] Could not detect display environment");
            eprintln!("[Error] Ensure WAYLAND_DISPLAY or DISPLAY is set");
            std::process::exit(1);
        }
    }

    Ok(RunOutcome::Exit)
}

// === Tests ===


#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;
