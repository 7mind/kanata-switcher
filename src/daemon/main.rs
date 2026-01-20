use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use futures_util::StreamExt;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon as SniIcon, MenuItem, Status as SniStatus, ToolTip, Tray, TrayService};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::fd::AsFd;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::TcpStream as TokioTcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{Mutex as TokioMutex, oneshot, watch};
use wayland_client::{
    Connection as WaylandConnection, Dispatch, Proxy, QueueHandle,
    backend::{ObjectId, WaylandError},
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use x11rb::connection::Connection as X11Connection;
use x11rb::protocol::Event as X11Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as X11ConnectionExt, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;
use zbus::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

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
        use crate::cosmic_workspace::__interfaces::*;
        use wayland_client::protocol::__interfaces::*;
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
    zcosmic_workspace_group_handle_v1::ZcosmicWorkspaceGroupHandleV1,
    zcosmic_workspace_handle_v1::ZcosmicWorkspaceHandleV1,
    zcosmic_workspace_manager_v1::ZcosmicWorkspaceManagerV1,
};

const GNOME_EXTENSION_UUID: &str = "kanata-switcher@7mind.io";
const GSETTINGS_SCHEMA_ID: &str = "org.gnome.shell.extensions.kanata-switcher";
const GSETTINGS_FOCUS_ONLY_KEY: &str = "show-focus-layer-only";
const GSETTINGS_COMPILED_FILENAME: &str = "gschemas.compiled";
const XDG_DATA_DIRS_FALLBACK: &str = "/usr/local/share:/usr/share";
const NIXOS_SYSTEM_PROFILE: &str = "/run/current-system/sw/share";
const NIXOS_DEFAULT_PROFILE: &str = "/nix/var/nix/profiles/default/share";
const NIXOS_PER_USER_PROFILE_PREFIX: &str = "/etc/profiles/per-user";
const NIXOS_PER_USER_PROFILE_ALT_PREFIX: &str = "/nix/var/nix/profiles/per-user";
const DBUS_NAME: &str = "com.github.kanata.Switcher";
const DBUS_PATH: &str = "/com/github/kanata/Switcher";
const DBUS_INTERFACE: &str = "com.github.kanata.Switcher";
const GNOME_FOCUS_OBJECT_PATH: &str = "/com/github/kanata/Switcher/Gnome";
const GNOME_FOCUS_INTERFACE: &str = "com.github.kanata.Switcher.Gnome";
const GNOME_FOCUS_METHOD: &str = "GetFocus";
const KDE_QUERY_INTERFACE: &str = "com.github.kanata.Switcher.KdeQuery";
const KDE_QUERY_METHOD: &str = "Focus";
const LOGIND_BUS_NAME: &str = "org.freedesktop.login1";
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";
const LOGIND_USER_INTERFACE: &str = "org.freedesktop.login1.User";
const LOGIND_ERROR_NO_SESSION_FOR_PID: &str = "org.freedesktop.login1.NoSessionForPID";
const LOGIND_EMPTY_OBJECT_PATH: &str = "/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlCommand {
    Restart,
    Pause,
    Unpause,
}

impl ControlCommand {
    fn dbus_method(self) -> &'static str {
        match self {
            ControlCommand::Restart => "Restart",
            ControlCommand::Pause => "Pause",
            ControlCommand::Unpause => "Unpause",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ControlCommand::Restart => "restart",
            ControlCommand::Pause => "pause",
            ControlCommand::Unpause => "unpause",
        }
    }
}

// === CLI ===

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TrayFocusOnly {
    True,
    False,
}

impl TrayFocusOnly {
    fn as_bool(self) -> bool {
        matches!(self, TrayFocusOnly::True)
    }

    fn as_arg(self) -> &'static str {
        match self {
            TrayFocusOnly::True => "true",
            TrayFocusOnly::False => "false",
        }
    }
}

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

    /// Disable the StatusNotifier (SNI) indicator on non-GNOME desktops
    #[arg(long)]
    no_indicator: bool,

    /// Override SNI focus-only mode (true/false). When set, GSettings is not read.
    #[arg(long, value_enum, value_name = "true|false")]
    indicator_focus_only: Option<TrayFocusOnly>,

    /// Install autostart desktop entry and exit
    #[arg(long, conflicts_with_all = ["uninstall_autostart", "restart", "pause", "unpause"])]
    install_autostart: bool,

    /// Uninstall autostart desktop entry and exit
    #[arg(long, conflicts_with_all = ["install_autostart", "restart", "pause", "unpause"])]
    uninstall_autostart: bool,

    /// Send Restart request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["pause", "unpause"])]
    restart: bool,

    /// Send Pause request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["restart", "unpause"])]
    pause: bool,

    /// Send Unpause request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["restart", "pause"])]
    unpause: bool,
}

const AUTOSTART_DESKTOP_FILENAME: &str = "kanata-switcher.desktop";
const AUTOSTART_PASSTHROUGH_OPTIONS: &[&str] = &[
    "port",
    "host",
    "config",
    "quiet",
    "quiet_focus",
    "install_gnome_extension",
    "no_install_gnome_extension",
    "no_indicator",
    "indicator_focus_only",
];
const AUTOSTART_ONESHOT_OPTIONS: &[&str] = &[
    "restart",
    "pause",
    "unpause",
    "install_autostart",
    "uninstall_autostart",
];

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

fn resolve_control_command(args: &Args) -> Option<ControlCommand> {
    if args.restart {
        return Some(ControlCommand::Restart);
    }
    if args.pause {
        return Some(ControlCommand::Pause);
    }
    if args.unpause {
        return Some(ControlCommand::Unpause);
    }
    None
}

fn resolve_binary_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let exe_path = env::current_exe()?;
    let canonical = exe_path.canonicalize()?;
    Ok(canonical)
}

fn autostart_dir() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        if xdg_config_home.is_empty() {
            return Err("XDG_CONFIG_HOME is empty".into());
        }
        return Ok(PathBuf::from(xdg_config_home).join("autostart"));
    }
    let home = env::var("HOME")?;
    if home.is_empty() {
        return Err("HOME is empty".into());
    }
    Ok(PathBuf::from(home).join(".config").join("autostart"))
}

fn autostart_desktop_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    Ok(autostart_dir()?.join(AUTOSTART_DESKTOP_FILENAME))
}

fn escape_desktop_exec_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '%' => escaped.push_str("%%"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn build_autostart_desktop_content(exec_path: &Path, exec_args: &[String]) -> String {
    let exec_path_str = exec_path
        .to_str()
        .expect("autostart exec path contains invalid UTF-8");
    let mut exec_parts = Vec::with_capacity(exec_args.len() + 1);
    exec_parts.push(escape_desktop_exec_arg(exec_path_str));
    for arg in exec_args {
        exec_parts.push(escape_desktop_exec_arg(arg));
    }
    let exec_line = exec_parts.join(" ");
    format!(
        "[Desktop Entry]\nType=Application\nName=Kanata Switcher\nExec={}\nTryExec={}\nX-GNOME-Autostart-enabled=true\n",
        exec_line,
        escape_desktop_exec_arg(exec_path_str)
    )
}

fn autostart_passthrough_args(matches: &ArgMatches, args: &Args) -> Vec<String> {
    use clap::parser::ValueSource;

    let mut exec_args = Vec::new();

    for &name in AUTOSTART_PASSTHROUGH_OPTIONS {
        if matches.value_source(name) != Some(ValueSource::CommandLine) {
            continue;
        }
        match name {
            "port" => {
                exec_args.push("-p".to_string());
                exec_args.push(args.port.to_string());
            }
            "host" => {
                exec_args.push("-H".to_string());
                exec_args.push(args.host.clone());
            }
            "config" => {
                let config = args
                    .config
                    .as_ref()
                    .expect("config missing after command-line input");
                exec_args.push("-c".to_string());
                exec_args.push(config.to_string_lossy().to_string());
            }
            "quiet" => {
                exec_args.push("-q".to_string());
            }
            "quiet_focus" => {
                exec_args.push("--quiet-focus".to_string());
            }
            "install_gnome_extension" => {
                exec_args.push("--install-gnome-extension".to_string());
            }
            "no_install_gnome_extension" => {
                exec_args.push("--no-install-gnome-extension".to_string());
            }
            "no_indicator" => {
                exec_args.push("--no-indicator".to_string());
            }
            "indicator_focus_only" => {
                let value = args
                    .indicator_focus_only
                    .expect("indicator_focus_only missing after command-line input");
                exec_args.push("--indicator-focus-only".to_string());
                exec_args.push(value.as_arg().to_string());
            }
            _ => {
                panic!("autostart passthrough option missing handler: {}", name);
            }
        }
    }

    exec_args
}

fn install_autostart_desktop(
    matches: &ArgMatches,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for option in AUTOSTART_ONESHOT_OPTIONS {
        if AUTOSTART_PASSTHROUGH_OPTIONS.contains(option) {
            return Err(format!("autostart option lists overlap: {}", option).into());
        }
    }
    let exec_path = resolve_binary_path()?;
    let exec_args = autostart_passthrough_args(matches, args);
    let content = build_autostart_desktop_content(&exec_path, &exec_args);

    let autostart_dir = autostart_dir()?;
    std::fs::create_dir_all(&autostart_dir)?;
    let desktop_path = autostart_dir.join(AUTOSTART_DESKTOP_FILENAME);

    std::fs::write(&desktop_path, content)?;
    println!("[Autostart] Installed {}", desktop_path.display());
    Ok(())
}

fn uninstall_autostart_desktop() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let desktop_path = autostart_desktop_path()?;
    if !desktop_path.exists() {
        return Err(format!("autostart entry not found: {}", desktop_path.display()).into());
    }
    std::fs::remove_file(&desktop_path)?;
    println!("[Autostart] Removed {}", desktop_path.display());
    Ok(())
}

async fn send_control_command(
    command: ControlCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    send_control_command_with_connection(&connection, command).await?;
    println!(
        "[Control] Sent {} request to running daemon",
        command.label()
    );
    Ok(())
}

async fn send_control_command_with_connection(
    connection: &Connection,
    command: ControlCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    connection
        .call_method(
            Some(DBUS_NAME),
            DBUS_PATH,
            Some(DBUS_INTERFACE),
            command.dbus_method(),
            &(),
        )
        .await?;
    Ok(())
}

// === Config ===

/// A rule for matching windows and triggering actions.
/// At least one of `layer`, `virtual_key`, or `raw_vk_action` should be specified.
#[derive(Debug, Clone, Deserialize)]
struct Rule {
    class: Option<String>,
    title: Option<String>,
    /// Layer to switch to when switching to a native terminal (VT)
    on_native_terminal: Option<String>,
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

#[derive(Debug, Clone)]
struct NativeTerminalRule {
    layer: String,
    virtual_key: Option<String>,
    raw_vk_action: Vec<(String, String)>,
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
    native_terminal_rule: Option<NativeTerminalRule>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WindowInfo {
    class: String,
    title: String,
    #[serde(default)]
    is_native_terminal: bool,
}

fn load_config(config_path: Option<&Path>) -> Config {
    let path = config_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let xdg_config = env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap().join(".config"));
        xdg_config.join("kanata").join("kanata-switcher.json")
    });

    if !path.exists() {
        eprintln!("[Config] Error: Config file not found: {}", path.display());
        eprintln!();
        eprintln!("Example config:");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Vec<ConfigEntry>>(&content) {
            Ok(entries) => {
                let mut rules = Vec::new();
                let mut default_layer: Option<String> = None;
                let mut native_terminal_rule: Option<NativeTerminalRule> = None;

                for entry in entries {
                    match entry {
                        ConfigEntry::Default { default } => {
                            if default_layer.is_some() {
                                eprintln!(
                                    "[Config] Error: multiple 'default' entries found, only one allowed"
                                );
                                std::process::exit(1);
                            }
                            default_layer = Some(default);
                        }
                        ConfigEntry::Rule(rule) => {
                            if let Some(layer) = rule.on_native_terminal.clone() {
                                if rule.class.is_some() || rule.title.is_some() {
                                    eprintln!(
                                        "[Config] Error: 'on_native_terminal' cannot be combined with 'class' or 'title'"
                                    );
                                    std::process::exit(1);
                                }
                                if rule.layer.is_some() {
                                    eprintln!(
                                        "[Config] Error: 'on_native_terminal' cannot be combined with 'layer'"
                                    );
                                    std::process::exit(1);
                                }
                                if native_terminal_rule.is_some() {
                                    eprintln!(
                                        "[Config] Error: multiple 'on_native_terminal' rules found, only one allowed"
                                    );
                                    std::process::exit(1);
                                }
                                native_terminal_rule = Some(NativeTerminalRule {
                                    layer,
                                    virtual_key: rule.virtual_key.clone(),
                                    raw_vk_action: rule.raw_vk_action.clone().unwrap_or_default(),
                                });
                            } else {
                                rules.push(rule);
                            }
                        }
                    }
                }

                println!(
                    "[Config] Loaded {} rules from {}",
                    rules.len(),
                    path.display()
                );

                Config {
                    rules,
                    default_layer,
                    native_terminal_rule,
                }
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

const NATIVE_TERMINAL_RULE_INDEX: usize = usize::MAX;

#[derive(Debug)]
struct FocusHandler {
    rules: Vec<Rule>,
    native_terminal_rule: Option<NativeTerminalRule>,
    last_class: String,
    last_title: String,
    last_matched_rules: Vec<usize>,
    last_effective_layer: String,
    /// Currently held virtual keys, in order they were pressed (top-to-bottom rule order)
    current_virtual_keys: Vec<String>,
    quiet_focus: bool,
}

impl FocusHandler {
    fn new(
        rules: Vec<Rule>,
        native_terminal_rule: Option<NativeTerminalRule>,
        quiet_focus: bool,
    ) -> Self {
        Self {
            rules,
            native_terminal_rule,
            last_class: String::new(),
            last_title: String::new(),
            last_matched_rules: Vec::new(),
            last_effective_layer: String::new(),
            current_virtual_keys: Vec::new(),
            quiet_focus,
        }
    }

    /// Handle a focus change event. Returns actions to execute.
    /// With fallthrough, ALL matching actions are collected and executed in order.
    /// All matched virtual_keys are pressed and held simultaneously.
    fn handle(&mut self, win: &WindowInfo, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();

        if win.is_native_terminal {
            return self.handle_native_terminal(default_layer);
        }

        // Handle unfocused state (no window has focus)
        if win.class.is_empty() && win.title.is_empty() {
            return self.handle_unfocused(default_layer);
        }

        if !self.quiet_focus {
            println!("[Focus] class=\"{}\" title=\"{}\"", win.class, win.title);
        }

        // Match rules with fallthrough support
        struct MatchedRule {
            index: usize,
            layer: Option<String>,
            virtual_key: Option<String>,
            raw_vk_actions: Vec<(String, String)>,
        }

        let mut matched_rules: Vec<MatchedRule> = Vec::new();

        for (index, rule) in self.rules.iter().enumerate() {
            if match_pattern(rule.class.as_deref(), &win.class)
                && match_pattern(rule.title.as_deref(), &win.title)
            {
                matched_rules.push(MatchedRule {
                    index,
                    layer: rule.layer.clone(),
                    virtual_key: rule.virtual_key.clone(),
                    raw_vk_actions: rule.raw_vk_action.clone().unwrap_or_default(),
                });

                if !rule.fallthrough {
                    break;
                }
            }
        }

        let matched_indices: Vec<usize> = matched_rules.iter().map(|rule| rule.index).collect();

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
            if !default_layer.is_empty() && self.last_effective_layer != default_layer {
                result
                    .actions
                    .push(FocusAction::ChangeLayer(default_layer.to_string()));
            }
            result.new_managed_vks = Vec::new();
            self.last_effective_layer = default_layer.to_string();
        } else {
            let matched_changed = matched_indices != self.last_matched_rules;
            let mut matched_layers: Vec<String> = Vec::new();
            for matched in &matched_rules {
                if let Some(layer) = matched.layer.clone() {
                    matched_layers.push(layer);
                }
            }
            let new_rules: Vec<usize> = matched_indices
                .iter()
                .cloned()
                .filter(|idx| !self.last_matched_rules.contains(idx))
                .collect();

            // Process matched rules in order, building action list
            for matched in matched_rules {
                let is_new = new_rules.contains(&matched.index);
                if is_new {
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
            }

            if matched_changed {
                if let Some(new_layer) = matched_layers.last().cloned() {
                    if self.last_effective_layer != new_layer {
                        let has_new_layer = result.actions.iter().rev().find_map(|action| {
                            if let FocusAction::ChangeLayer(layer) = action {
                                Some(layer == &new_layer)
                            } else {
                                None
                            }
                        });
                        if has_new_layer != Some(true) {
                            result
                                .actions
                                .push(FocusAction::ChangeLayer(new_layer.clone()));
                        }
                    }
                    self.last_effective_layer = new_layer;
                }
            }

            result.new_managed_vks = new_vks;
        }

        // Update state
        self.last_class = win.class.clone();
        self.last_title = win.title.clone();
        self.last_matched_rules = matched_indices;
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
        self.last_matched_rules.clear();
        self.last_effective_layer.clear();
        self.current_virtual_keys.clear();
    }

    fn handle_unfocused(&mut self, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();
        if !self.quiet_focus {
            println!("[Focus] No window focused");
        }
        // Release all active virtual keys in reverse order (bottom-to-top)
        for vk in self.current_virtual_keys.iter().rev() {
            result.actions.push(FocusAction::ReleaseVk(vk.clone()));
        }
        // Switch to default layer
        if !default_layer.is_empty() && self.last_effective_layer != default_layer {
            result
                .actions
                .push(FocusAction::ChangeLayer(default_layer.to_string()));
        }
        result.new_managed_vks = Vec::new();
        self.current_virtual_keys = Vec::new();
        self.last_matched_rules.clear();
        self.last_effective_layer = default_layer.to_string();
        self.last_class.clear();
        self.last_title.clear();
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn handle_native_terminal(&mut self, default_layer: &str) -> Option<FocusActions> {
        let Some(rule) = self.native_terminal_rule.clone() else {
            return self.handle_native_terminal_without_rule(default_layer);
        };

        if !self.quiet_focus {
            println!("[Focus] Native terminal active");
        }

        let mut result = FocusActions::default();
        let mut new_vks = Vec::new();

        if let Some(vk) = rule.virtual_key.clone() {
            new_vks.push(vk);
        }

        for vk in self.current_virtual_keys.iter().rev() {
            if !new_vks.contains(vk) {
                result.actions.push(FocusAction::ReleaseVk(vk.clone()));
            }
        }

        let matched_indices = vec![NATIVE_TERMINAL_RULE_INDEX];
        let is_new = self.last_matched_rules != matched_indices;

        if is_new {
            if !rule.layer.is_empty() && self.last_effective_layer != rule.layer {
                result
                    .actions
                    .push(FocusAction::ChangeLayer(rule.layer.clone()));
            }
            if let Some(vk) = rule.virtual_key {
                if !self.current_virtual_keys.contains(&vk) {
                    result.actions.push(FocusAction::PressVk(vk));
                }
            }
            for (name, action) in rule.raw_vk_action {
                result.actions.push(FocusAction::RawVkAction(name, action));
            }
        }

        result.new_managed_vks = new_vks;
        self.last_matched_rules = matched_indices;
        self.last_effective_layer = rule.layer;
        self.current_virtual_keys = result.new_managed_vks.clone();
        self.last_class.clear();
        self.last_title.clear();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn handle_native_terminal_without_rule(&mut self, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();
        if !self.quiet_focus {
            println!("[Focus] Native terminal active");
        }
        for vk in self.current_virtual_keys.iter().rev() {
            result.actions.push(FocusAction::ReleaseVk(vk.clone()));
        }
        if !default_layer.is_empty() && self.last_effective_layer != default_layer {
            result
                .actions
                .push(FocusAction::ChangeLayer(default_layer.to_string()));
        }
        result.new_managed_vks = Vec::new();
        self.current_virtual_keys = Vec::new();
        self.last_matched_rules.clear();
        self.last_effective_layer = default_layer.to_string();
        self.last_class.clear();
        self.last_title.clear();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
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

#[derive(Clone, Debug)]
struct ShutdownHandle {
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

impl ShutdownHandle {
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

async fn wait_for_restart_or_shutdown(
    restart_handle: &RestartHandle,
    shutdown_handle: &ShutdownHandle,
) -> RunOutcome {
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();

    if *shutdown_receiver.borrow() {
        return RunOutcome::Exit;
    }
    if *restart_receiver.borrow() {
        return RunOutcome::Restart;
    }

    tokio::select! {
        _ = shutdown_receiver.changed() => RunOutcome::Exit,
        _ = restart_receiver.changed() => {
            if *shutdown_receiver.borrow() {
                RunOutcome::Exit
            } else {
                RunOutcome::Restart
            }
        }
    }
}

// === SNI Indicator ===

const SNI_DEFAULT_SHOW_FOCUS_ONLY: bool = true;
const SNI_ICON_SIZE: usize = 24;
const SNI_GLYPH_SIZE: usize = 8;
const SNI_GLYPH_Y: usize = (SNI_ICON_SIZE - SNI_GLYPH_SIZE) / 2;
const SNI_GLYPH_MARGIN: usize = 3;
const SNI_LAYER_X_SINGLE: usize = (SNI_ICON_SIZE - SNI_GLYPH_SIZE) / 2;
const SNI_LAYER_X_DOUBLE: usize = SNI_GLYPH_MARGIN;
const SNI_VK_X_DOUBLE: usize = SNI_ICON_SIZE - SNI_GLYPH_SIZE - SNI_GLYPH_MARGIN;
const SNI_COLOR_LAYER: [u8; 4] = [255, 255, 255, 255];
const SNI_COLOR_VK: [u8; 4] = [255, 0, 255, 255];
const SNI_INFINITY_SYMBOL: char = '∞';
const SNI_MAX_VK_COUNT_DIGIT: usize = 9;
const SNI_MIN_MULTI_VK_COUNT: usize = 2;
const SNI_INDICATOR_TITLE: &str = "Kanata Switcher";
const SNI_INDICATOR_ID: &str = "kanata-switcher";

trait GsettingsBackend: Send + Sync {
    fn get_bool(&self, schema_dir: Option<&Path>, schema: &str, key: &str) -> Result<bool, String>;
    fn set_bool(
        &self,
        schema_dir: Option<&Path>,
        schema: &str,
        key: &str,
        value: bool,
    ) -> Result<(), String>;
}

struct ShellGsettingsBackend;

impl GsettingsBackend for ShellGsettingsBackend {
    fn get_bool(&self, schema_dir: Option<&Path>, schema: &str, key: &str) -> Result<bool, String> {
        gsettings_get_bool(schema_dir, schema, key)
    }

    fn set_bool(
        &self,
        schema_dir: Option<&Path>,
        schema: &str,
        key: &str,
        value: bool,
    ) -> Result<(), String> {
        gsettings_set_bool(schema_dir, schema, key, value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GsettingsSchemaTarget {
    Default,
    Dir(PathBuf),
}

impl GsettingsSchemaTarget {
    fn as_option(&self) -> Option<&Path> {
        match self {
            GsettingsSchemaTarget::Default => None,
            GsettingsSchemaTarget::Dir(dir) => Some(dir.as_path()),
        }
    }
}

struct SniSettingsStore {
    schema_dirs: Vec<PathBuf>,
    active_schema: Option<GsettingsSchemaTarget>,
    available: bool,
    backend: Box<dyn GsettingsBackend>,
}

impl SniSettingsStore {
    fn new() -> Self {
        Self {
            schema_dirs: find_gsettings_schema_dirs(),
            active_schema: None,
            available: true,
            backend: Box::new(ShellGsettingsBackend),
        }
    }

    #[cfg(test)]
    fn with_backend(schema_dirs: Vec<PathBuf>, backend: Box<dyn GsettingsBackend>) -> Self {
        Self {
            schema_dirs,
            active_schema: None,
            available: true,
            backend,
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            schema_dirs: Vec::new(),
            active_schema: None,
            available: false,
            backend: Box::new(ShellGsettingsBackend),
        }
    }

    fn schema_targets(&self) -> Vec<GsettingsSchemaTarget> {
        let mut targets = Vec::new();
        if let Some(active) = self.active_schema.clone() {
            targets.push(active);
        }
        targets.push(GsettingsSchemaTarget::Default);
        for dir in &self.schema_dirs {
            let target = GsettingsSchemaTarget::Dir(dir.clone());
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        targets
    }

    fn read_focus_only(&mut self) -> Option<bool> {
        if !self.available {
            return None;
        }
        let mut last_error: Option<String> = None;
        for target in self.schema_targets() {
            match self.backend.get_bool(
                target.as_option(),
                GSETTINGS_SCHEMA_ID,
                GSETTINGS_FOCUS_ONLY_KEY,
            ) {
                Ok(value) => {
                    self.active_schema = Some(target);
                    return Some(value);
                }
                Err(error) => {
                    if is_gsettings_unavailable(&error) {
                        self.available = false;
                        last_error = Some(error);
                        break;
                    }
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            eprintln!("[SNI] GSettings read failed: {}", error);
        }
        None
    }

    fn write_focus_only(&mut self, value: bool) {
        if !self.available {
            return;
        }
        let mut last_error: Option<String> = None;
        for target in self.schema_targets() {
            match self.backend.set_bool(
                target.as_option(),
                GSETTINGS_SCHEMA_ID,
                GSETTINGS_FOCUS_ONLY_KEY,
                value,
            ) {
                Ok(()) => {
                    self.active_schema = Some(target);
                    return;
                }
                Err(error) => {
                    if is_gsettings_unavailable(&error) {
                        self.available = false;
                        last_error = Some(error);
                        break;
                    }
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            eprintln!("[SNI] GSettings write failed: {}", error);
        }
    }
}

struct MenuRefresh {
    sender: watch::Sender<u64>,
    version: u64,
}

impl MenuRefresh {
    fn new() -> (Self, watch::Receiver<u64>) {
        let (sender, receiver) = watch::channel(0u64);
        (
            Self {
                sender,
                version: 0,
            },
            receiver,
        )
    }

    fn notify(&mut self) {
        self.version += 1;
        self.sender.send_replace(self.version);
    }
}

#[derive(Clone, Debug)]
struct SniIndicatorState {
    last_status: StatusSnapshot,
    focus_status: StatusSnapshot,
    paused: bool,
    show_focus_only: bool,
    menu_revision: u64,
}

impl SniIndicatorState {
    fn new(initial: StatusSnapshot, show_focus_only: bool) -> Self {
        Self {
            last_status: initial.clone(),
            focus_status: initial,
            paused: false,
            show_focus_only,
            menu_revision: 0,
        }
    }

    fn update_status(&mut self, snapshot: StatusSnapshot) {
        if snapshot.layer_source == LayerSource::Focus {
            self.focus_status = snapshot.clone();
        }
        self.last_status = snapshot;
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn toggle_focus_only(&mut self) {
        self.show_focus_only = !self.show_focus_only;
    }

    fn focus_only_enabled(&self) -> bool {
        self.show_focus_only
    }

    fn bump_menu_revision(&mut self) {
        self.menu_revision = self.menu_revision.wrapping_add(1);
    }

    fn display_status(&self) -> StatusSnapshot {
        if self.paused {
            return self.last_status.clone();
        }
        if self.show_focus_only {
            return self.focus_status.clone();
        }
        self.last_status.clone()
    }
}

#[derive(Clone)]
struct SniLocalControl {
    runtime_handle: tokio::runtime::Handle,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    restart_handle: RestartHandle,
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
}

#[derive(Clone)]
struct SniDbusControl {
    runtime_handle: tokio::runtime::Handle,
    connection: Connection,
    restart_handle: RestartHandle,
}

#[derive(Clone)]
enum SniControl {
    Local(SniLocalControl),
    Dbus(SniDbusControl),
}

trait SniControlOps: Send + Sync {
    fn restart(&self);
    fn pause(&self);
    fn unpause(&self);
}

impl SniControlOps for SniControl {
    fn restart(&self) {
        println!("[SNI] Restart requested");
        match self {
            SniControl::Local(control) => {
                control.restart_handle.request();
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        ControlCommand::Restart,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send restart: {}", error);
                    }
                });
                control.restart_handle.request();
            }
        }
    }

    fn pause(&self) {
        println!("[SNI] Pause requested");
        match self {
            SniControl::Local(control) => {
                pause_daemon(
                    &control.pause_broadcaster,
                    &control.handler,
                    &control.status_broadcaster,
                    &control.kanata,
                    &control.runtime_handle,
                    "via SNI",
                );
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        ControlCommand::Pause,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send pause: {}", error);
                    }
                });
            }
        }
    }

    fn unpause(&self) {
        println!("[SNI] Unpause requested");
        match self {
            SniControl::Local(control) => {
                unpause_daemon(
                    control.env,
                    control.connection.clone(),
                    control.is_kde6,
                    &control.pause_broadcaster,
                    &control.handler,
                    &control.status_broadcaster,
                    &control.kanata,
                    &control.runtime_handle,
                    "via SNI",
                );
            }
            SniControl::Dbus(control) => {
                control.runtime_handle.block_on(async {
                    if let Err(error) = send_control_command_with_connection(
                        &control.connection,
                        ControlCommand::Unpause,
                    )
                    .await
                    {
                        eprintln!("[SNI] Failed to send unpause: {}", error);
                    }
                });
            }
        }
    }
}

struct SniIndicator {
    state: SniIndicatorState,
    control: Arc<dyn SniControlOps>,
    settings: SniSettingsStore,
    menu_refresh: MenuRefresh,
}

impl SniIndicator {
    fn update_status(&mut self, snapshot: StatusSnapshot) {
        self.state.update_status(snapshot);
    }

    fn set_paused(&mut self, paused: bool) {
        self.state.set_paused(paused);
    }

    fn toggle_focus_only(&mut self) {
        self.state.toggle_focus_only();
        let show_focus_only = self.state.focus_only_enabled();
        self.settings.write_focus_only(show_focus_only);
        self.menu_refresh.notify();
    }

    fn request_pause(&self) {
        if self.state.paused {
            self.control.unpause();
        } else {
            self.control.pause();
        }
    }

    fn request_restart(&self) {
        self.control.restart();
    }

    fn format_layer_letter(layer_name: &str) -> String {
        let trimmed = layer_name.trim();
        if trimmed.is_empty() {
            return "?".to_string();
        }
        trimmed
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    fn format_virtual_keys(virtual_keys: &[String]) -> String {
        let count = virtual_keys.len();
        if count == 0 {
            return String::new();
        }
        if count == 1 {
            let name = virtual_keys[0].trim();
            if name.is_empty() {
                return String::new();
            }
            return name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
        }
        if count < SNI_MIN_MULTI_VK_COUNT {
            return String::new();
        }
        if count > SNI_MAX_VK_COUNT_DIGIT {
            return SNI_INFINITY_SYMBOL.to_string();
        }
        count.to_string()
    }

    fn glyph_for_char(ch: char) -> [u8; 8] {
        const INFINITY_GLYPH: [u8; 8] = [
            0b00000000, 0b00111100, 0b01000010, 0b10011001, 0b10011001, 0b01000010, 0b00111100,
            0b00000000,
        ];
        if ch == SNI_INFINITY_SYMBOL {
            return INFINITY_GLYPH;
        }
        BASIC_FONTS
            .get(ch)
            .unwrap_or_else(|| BASIC_FONTS.get('?').unwrap_or([0; 8]))
    }

    fn draw_glyph(
        buffer: &mut [u8],
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        glyph: [u8; 8],
        color: [u8; 4],
    ) {
        for (row_index, row) in glyph.iter().enumerate() {
            let dest_y = y + row_index;
            if dest_y >= height {
                continue;
            }
            for col_index in 0..SNI_GLYPH_SIZE {
                let dest_x = x + col_index;
                if dest_x >= width {
                    continue;
                }
                if row & (1 << col_index) == 0 {
                    continue;
                }
                let offset = (dest_y * width + dest_x) * 4;
                buffer[offset] = color[0];
                buffer[offset + 1] = color[1];
                buffer[offset + 2] = color[2];
                buffer[offset + 3] = color[3];
            }
        }
    }

    fn render_icon(layer_text: &str, vk_text: &str) -> SniIcon {
        let mut buffer = vec![0u8; SNI_ICON_SIZE * SNI_ICON_SIZE * 4];
        let layer_char = layer_text.chars().next().unwrap_or('?');
        let vk_char = vk_text.chars().next();

        if let Some(vk_char) = vk_char {
            let layer_glyph = Self::glyph_for_char(layer_char);
            let vk_glyph = Self::glyph_for_char(vk_char);
            Self::draw_glyph(
                &mut buffer,
                SNI_ICON_SIZE,
                SNI_ICON_SIZE,
                SNI_LAYER_X_DOUBLE,
                SNI_GLYPH_Y,
                layer_glyph,
                SNI_COLOR_LAYER,
            );
            Self::draw_glyph(
                &mut buffer,
                SNI_ICON_SIZE,
                SNI_ICON_SIZE,
                SNI_VK_X_DOUBLE,
                SNI_GLYPH_Y,
                vk_glyph,
                SNI_COLOR_VK,
            );
        } else {
            let layer_glyph = Self::glyph_for_char(layer_char);
            Self::draw_glyph(
                &mut buffer,
                SNI_ICON_SIZE,
                SNI_ICON_SIZE,
                SNI_LAYER_X_SINGLE,
                SNI_GLYPH_Y,
                layer_glyph,
                SNI_COLOR_LAYER,
            );
        }

        SniIcon {
            width: SNI_ICON_SIZE as i32,
            height: SNI_ICON_SIZE as i32,
            data: buffer,
        }
    }

    fn display_strings(&self) -> (String, String) {
        let status = self.state.display_status();
        let layer_text = Self::format_layer_letter(&status.layer);
        let vk_text = Self::format_virtual_keys(&status.virtual_keys);
        (layer_text, vk_text)
    }

    fn tooltip_text(&self) -> String {
        let status = self.state.display_status();
        if status.virtual_keys.is_empty() {
            return format!("Layer: {}", status.layer);
        }
        format!(
            "Layer: {}\nVirtual keys: {}",
            status.layer,
            status.virtual_keys.join(", ")
        )
    }
}

impl Tray for SniIndicator {
    fn id(&self) -> String {
        SNI_INDICATOR_ID.to_string()
    }

    fn title(&self) -> String {
        SNI_INDICATOR_TITLE.to_string()
    }

    fn status(&self) -> SniStatus {
        SniStatus::Active
    }

    fn icon_pixmap(&self) -> Vec<SniIcon> {
        let (layer_text, vk_text) = self.display_strings();
        vec![Self::render_icon(&layer_text, &vk_text)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: SNI_INDICATOR_TITLE.to_string(),
            description: self.tooltip_text(),
            ..ToolTip::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Checkmark(CheckmarkItem {
                label: "Pause".to_string(),
                checked: self.state.paused,
                activate: Box::new(|this| {
                    this.request_pause();
                }),
                ..CheckmarkItem::default()
            }),
            MenuItem::Checkmark(CheckmarkItem {
                label: "Show app layer only".to_string(),
                checked: self.state.show_focus_only,
                activate: Box::new(|this| {
                    this.toggle_focus_only();
                }),
                ..CheckmarkItem::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Restart".to_string(),
                activate: Box::new(|this| {
                    this.request_restart();
                }),
                ..StandardItem::default()
            }),
        ]
    }

    fn watcher_online(&self) {
        println!("[SNI] StatusNotifierWatcher online");
    }

    fn watcher_offine(&self) -> bool {
        eprintln!("[SNI] StatusNotifierWatcher offline");
        true
    }
}

fn resolve_sni_focus_only(
    override_value: Option<TrayFocusOnly>,
    settings: &mut SniSettingsStore,
) -> bool {
    if let Some(value) = override_value {
        return value.as_bool();
    }
    settings
        .read_focus_only()
        .unwrap_or(SNI_DEFAULT_SHOW_FOCUS_ONLY)
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

async fn update_status_for_focus(
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    win: &WindowInfo,
    kanata: &KanataClient,
    default_layer: &str,
) -> Option<FocusActions> {
    let (actions, virtual_keys, focus_layer) = {
        let mut handler = handler.lock().unwrap();
        let actions = handler.handle(win, default_layer);
        let virtual_keys = handler.current_virtual_keys();
        let focus_layer = actions
            .as_ref()
            .and_then(|focus_actions| extract_focus_layer(focus_actions));
        (actions, virtual_keys, focus_layer)
    };

    status_broadcaster.update_virtual_keys(virtual_keys);
    if let Some(layer) = focus_layer {
        if let Some(resolved_layer) = kanata.resolve_layer_name(&layer, false).await {
            status_broadcaster.update_focus_layer(resolved_layer);
        }
    }

    actions
}

async fn handle_focus_event(
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    pause_broadcaster: &PauseBroadcaster,
    win: &WindowInfo,
    kanata: &KanataClient,
    default_layer: &str,
) -> Option<FocusActions> {
    if pause_broadcaster.is_paused() {
        return None;
    }
    update_status_for_focus(handler, status_broadcaster, win, kanata, default_layer).await
}

fn native_terminal_window() -> WindowInfo {
    WindowInfo {
        class: String::new(),
        title: String::new(),
        is_native_terminal: true,
    }
}

#[derive(Clone, Copy, Debug)]
struct RawFdWatcher {
    fd: RawFd,
}

impl RawFdWatcher {
    fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl AsRawFd for RawFdWatcher {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

fn query_wayland_active_window() -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(test)]
    {
        WAYLAND_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    }
    let connection = WaylandConnection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<WaylandState>(&connection)?;
    let mut state = WaylandState::default();

    if globals
        .bind::<ZwlrForeignToplevelManagerV1, _, _>(&queue.handle(), 1..=3, ())
        .is_err()
        && globals
            .bind::<ZcosmicToplevelInfoV1, _, _>(&queue.handle(), 1..=1, ())
            .is_err()
    {
        return Err(
            "No supported toplevel protocol (wlr-foreign-toplevel or cosmic-toplevel-info)".into(),
        );
    }

    for _ in 0..5 {
        queue.roundtrip(&mut state)?;
        if state.active_window.is_some() {
            break;
        }
    }
    Ok(state.get_active_window())
}

#[cfg(test)]
fn wayland_query_count() -> usize {
    WAYLAND_QUERY_COUNTER.load(Ordering::SeqCst)
}

fn query_x11_active_window() -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new()?;
    Ok(state.get_active_window())
}

static KDE_QUERY_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static WAYLAND_QUERY_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct KdeFocusQueryService {
    sender: TokioMutex<Option<oneshot::Sender<WindowInfo>>>,
}

#[zbus::interface(name = "com.github.kanata.Switcher.KdeQuery")]
impl KdeFocusQueryService {
    #[allow(non_snake_case)]
    async fn Focus(&self, window_class: &str, window_title: &str) {
        let win = WindowInfo {
            class: window_class.to_string(),
            title: window_title.to_string(),
            is_native_terminal: false,
        };
        let mut sender = self.sender.lock().await;
        if let Some(tx) = sender.take() {
            let _ = tx.send(win);
        }
    }
}

fn kwin_script_object_path(
    script_num: i32,
    is_kde6: bool,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let path = if is_kde6 {
        format!("/Scripting/Script{}", script_num)
    } else {
        format!("/{}", script_num)
    };
    let obj_path: OwnedObjectPath = path.as_str().try_into()?;
    Ok(obj_path)
}

async fn load_kwin_script(
    connection: &Connection,
    script_path: &str,
    is_kde6: bool,
    cleanup_existing: bool,
) -> Result<(OwnedObjectPath, &'static str), Box<dyn std::error::Error + Send + Sync>> {
    if cleanup_existing {
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
    }

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
    let obj_path = kwin_script_object_path(script_num, is_kde6)?;
    Ok((obj_path, "org.kde.kwin.Script"))
}

fn build_kde_query_script(is_kde6: bool, bus_name: &str, object_path: &str) -> String {
    let active_window = if is_kde6 {
        "activeWindow"
    } else {
        "activeClient"
    };
    format!(
        r#"function reportFocus(client) {{
  callDBus(
    "{bus}",
    "{path}",
    "{iface}",
    "{method}",
    client ? (client.resourceClass || "") : "",
    client ? (client.caption || "") : ""
  );
}}
reportFocus(workspace.{active});
"#,
        bus = bus_name,
        path = object_path,
        iface = KDE_QUERY_INTERFACE,
        method = KDE_QUERY_METHOD,
        active = active_window
    )
}

async fn query_kde_focus(
    connection: &Connection,
    is_kde6: bool,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let unique_name = connection
        .unique_name()
        .ok_or("KDE focus query requires a unique DBus name")?;
    let query_id = KDE_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    let query_path = format!("/com/github/kanata/Switcher/KdeQuery{}", query_id);
    let (sender, receiver) = oneshot::channel();
    let service = KdeFocusQueryService {
        sender: TokioMutex::new(Some(sender)),
    };
    connection
        .object_server()
        .at(query_path.as_str(), service)
        .await?;

    let uid = unsafe { libc::getuid() };
    let script_path = format!("/tmp/kanata-switcher-kwin-query-{}-{}.js", uid, query_id);
    let script = build_kde_query_script(is_kde6, unique_name.as_str(), query_path.as_str());
    fs::write(&script_path, script)?;

    let (script_obj_path, script_interface) =
        load_kwin_script(connection, &script_path, is_kde6, false).await?;

    let _kwin_query_guard = KwinScriptGuard::new(
        connection.clone(),
        tokio::runtime::Handle::current(),
        script_path.clone(),
        script_obj_path.clone(),
        script_interface,
    );

    connection
        .call_method(
            Some("org.kde.KWin"),
            script_obj_path,
            Some(script_interface),
            "run",
            &(),
        )
        .await?;

    let win = tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .map_err(|_| "Timed out waiting for KDE focus callback")?
        .map_err(|_| "KDE focus callback sender dropped")?;

    Ok(win)
}

async fn query_gnome_focus(
    connection: &Connection,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let reply = connection
        .call_method(
            Some(GNOME_SHELL_BUS_NAME),
            GNOME_FOCUS_OBJECT_PATH,
            Some(GNOME_FOCUS_INTERFACE),
            GNOME_FOCUS_METHOD,
            &(),
        )
        .await?;
    let (class, title): (String, String) = reply.body().deserialize()?;
    Ok(WindowInfo {
        class,
        title,
        is_native_terminal: false,
    })
}

async fn query_focus_for_env(
    env: Environment,
    connection: Option<&Connection>,
    is_kde6: bool,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    match env {
        Environment::Gnome => {
            let conn = connection.expect("GNOME focus query requires session connection");
            query_gnome_focus(conn).await
        }
        Environment::Kde => {
            let conn = connection.expect("KDE focus query requires session connection");
            query_kde_focus(conn, is_kde6).await
        }
        Environment::Wayland => tokio::task::block_in_place(query_wayland_active_window),
        Environment::X11 => tokio::task::block_in_place(query_x11_active_window),
        Environment::Unknown => Ok(WindowInfo::default()),
    }
}

async fn apply_focus_for_env(
    env: Environment,
    connection: Option<&Connection>,
    is_kde6: bool,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    pause_broadcaster: &PauseBroadcaster,
    kanata: &KanataClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let win = query_focus_for_env(env, connection, is_kde6).await?;
    let default_layer = kanata.default_layer().await.unwrap_or_default();
    if let Some(actions) = handle_focus_event(
        handler,
        status_broadcaster,
        pause_broadcaster,
        &win,
        kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(kanata, actions).await;
    }
    Ok(())
}
async fn apply_session_focus(
    active: bool,
    env: Environment,
    connection: Option<&Connection>,
    is_kde6: bool,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    pause_broadcaster: &PauseBroadcaster,
    kanata: &KanataClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if active {
        return apply_focus_for_env(
            env,
            connection,
            is_kde6,
            handler,
            status_broadcaster,
            pause_broadcaster,
            kanata,
        )
        .await;
    }

    let win = native_terminal_window();
    let default_layer = kanata.default_layer().await.unwrap_or_default();
    if let Some(actions) = handle_focus_event(
        handler,
        status_broadcaster,
        pause_broadcaster,
        &win,
        kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(kanata, actions).await;
    }

    Ok(())
}

async fn resolve_logind_session_path(
    connection: &Connection,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let manager = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await?;

    if let Ok(session_id) = env::var("XDG_SESSION_ID") {
        println!("[Logind] Using XDG_SESSION_ID={}", session_id);
        let reply = manager.call_method("GetSession", &(session_id)).await?;
        let path = decode_logind_object_path_reply(&reply, "GetSession")?;
        println!("[Logind] Using session path: {}", path.as_str());
        return Ok(path);
    }
    println!("[Logind] XDG_SESSION_ID not set; resolving session via logind");

    let pid = std::process::id();
    match manager.call_method("GetSessionByPID", &(pid)).await {
        Ok(reply) => {
            let path = decode_logind_object_path_reply(&reply, "GetSessionByPID")?;
            println!("[Logind] Using session path: {}", path.as_str());
            Ok(path)
        }
        Err(error) => {
            if is_logind_no_session_error(&error) {
                return resolve_logind_display_session_path(&manager, connection, pid).await;
            }
            Err(error.into())
        }
    }
}

fn is_logind_no_session_error(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => name.as_ref() == LOGIND_ERROR_NO_SESSION_FOR_PID,
        _ => false,
    }
}

fn is_logind_empty_object_path(path: &OwnedObjectPath) -> bool {
    path.as_str() == LOGIND_EMPTY_OBJECT_PATH
}

fn parse_logind_object_path(
    value: OwnedValue,
    context: &str,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let debug_value = format!("{:?}", value);
    if let Ok(path) = OwnedObjectPath::try_from(value.try_clone()?) {
        return Ok(path);
    }
    if let Ok(structure) = Structure::try_from(value.try_clone()?) {
        if let Some(path) = parse_logind_object_path_from_structure(&structure) {
            return Ok(path);
        }
    }
    if let Ok(text) = String::try_from(value) {
        return OwnedObjectPath::try_from(text).map_err(|error| {
            format!(
                "logind {} returned invalid object path string: {}",
                context, error
            )
            .into()
        });
    }
    Err(format!(
        "logind {} returned unexpected value: {}",
        context, debug_value
    )
    .into())
}

fn parse_logind_object_path_from_structure(structure: &Structure<'_>) -> Option<OwnedObjectPath> {
    let fields = structure.fields();
    if fields.is_empty() {
        return None;
    }
    fields
        .iter()
        .find_map(|field| logind_object_path_from_value(field))
}

fn decode_logind_object_path_reply(
    reply: &zbus::Message,
    context: &str,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let body = reply.body();
    let signature = body.signature().to_string();
    match signature.as_str() {
        "o" => Ok(body.deserialize_unchecked::<OwnedObjectPath>()?),
        "s" => {
            let text = body.deserialize_unchecked::<String>()?;
            OwnedObjectPath::try_from(text).map_err(|error| {
                format!(
                    "logind {} returned invalid object path string: {}",
                    context, error
                )
                .into()
            })
        }
        "v" => {
            let value = body.deserialize::<OwnedValue>()?;
            parse_logind_object_path(value, context)
        }
        _ => {
            if signature.starts_with('(') {
                let structure = body.deserialize::<Structure>()?;
                return parse_logind_object_path_from_structure(&structure).ok_or_else(|| {
                    format!(
                        "logind {} returned unexpected structure: {}",
                        context, signature
                    )
                    .into()
                });
            }
            Err(format!(
                "logind {} returned unexpected signature: {}",
                context, signature
            )
            .into())
        }
    }
}

fn logind_object_path_from_value(value: &Value<'_>) -> Option<OwnedObjectPath> {
    match value {
        Value::ObjectPath(path) => Some(OwnedObjectPath::from(path.clone())),
        Value::Str(text) => OwnedObjectPath::try_from(text.as_str()).ok(),
        Value::Value(inner) => logind_object_path_from_value(inner),
        _ => None,
    }
}

async fn resolve_logind_display_session_path(
    manager: &zbus::Proxy<'_>,
    connection: &Connection,
    pid: u32,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let user_reply = manager.call_method("GetUserByPID", &(pid)).await?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")?;
    let user_proxy = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        user_path,
        LOGIND_USER_INTERFACE,
    )
    .await?;
    let display = parse_logind_object_path(
        user_proxy.get_property::<OwnedValue>("Display").await?,
        "User.Display",
    )?;
    if is_logind_empty_object_path(&display) {
        return Err("logind user has no display session".into());
    }
    println!("[Logind] Using display session path: {}", display.as_str());
    Ok(display)
}

async fn start_logind_session_monitor(
    env: Environment,
    session_connection: Option<Connection>,
    is_kde6: bool,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    kanata: KanataClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::system().await?;
    let session_path = resolve_logind_session_path(&connection).await?;
    let session_proxy = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        session_path.clone(),
        LOGIND_SESSION_INTERFACE,
    )
    .await?;
    let active: bool = session_proxy.get_property("Active").await?;

    if !active {
        apply_session_focus(
            false,
            env,
            session_connection.as_ref(),
            is_kde6,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await?;
    }

    let properties_proxy = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(LOGIND_BUS_NAME)?
        .path(session_path.clone())?
        .build()
        .await?;
    let mut signals = properties_proxy.receive_properties_changed().await?;

    let session_connection = session_connection.clone();
    tokio::spawn(async move {
        let mut last_active = active;
        while let Some(signal) = signals.next().await {
            let args = match signal.args() {
                Ok(args) => args,
                Err(error) => {
                    eprintln!(
                        "[Logind] Failed to parse PropertiesChanged signal: {}",
                        error
                    );
                    std::process::exit(1);
                }
            };
            let Some(value) = args.changed_properties.get("Active") else {
                continue;
            };
            let next_active = match value.downcast_ref::<bool>().ok() {
                Some(active_value) => active_value,
                None => {
                    eprintln!("[Logind] Failed to parse Active property");
                    std::process::exit(1);
                }
            };

            if next_active == last_active {
                continue;
            }
            last_active = next_active;

            if let Err(error) = apply_session_focus(
                next_active,
                env,
                session_connection.as_ref(),
                is_kde6,
                &handler,
                &status_broadcaster,
                &pause_broadcaster,
                &kanata,
            )
            .await
            {
                eprintln!("[Logind] Failed to apply session focus: {}", error);
                std::process::exit(1);
            }
        }
    });

    Ok(())
}

async fn start_logind_session_monitor_best_effort<F, Fut>(
    env: Environment,
    session_connection: Option<Connection>,
    is_kde6: bool,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    kanata: KanataClient,
    starter: F,
) -> bool
where
    F: FnOnce(
        Environment,
        Option<Connection>,
        bool,
        Arc<Mutex<FocusHandler>>,
        StatusBroadcaster,
        PauseBroadcaster,
        KanataClient,
    ) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    match starter(
        env,
        session_connection,
        is_kde6,
        handler,
        status_broadcaster,
        pause_broadcaster,
        kanata,
    )
    .await
    {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "[Logind] Disabled native terminal monitoring (startup failed): {}",
                error
            );
            false
        }
    }
}

fn pause_daemon(
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    runtime_handle: &tokio::runtime::Handle,
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
    let status_broadcaster = status_broadcaster.clone();
    let kanata = kanata.clone();
    runtime_handle.block_on(async move {
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

fn unpause_daemon(
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    runtime_handle: &tokio::runtime::Handle,
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
    let pause_broadcaster = pause_broadcaster.clone();
    let handler = handler.clone();
    let status_broadcaster = status_broadcaster.clone();
    let kanata = kanata.clone();
    runtime_handle.block_on(async move {
        kanata.unpause_connect().await;
        if let Err(error) = apply_focus_for_env(
            env,
            connection.as_ref(),
            is_kde6,
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &kanata,
        )
        .await
        {
            panic!("[Pause] Failed to refresh focus after unpause: {}", error);
        }
    });
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

    async fn resolve_layer_name(&self, layer_name: &str, warn_unknown: bool) -> Option<String> {
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

        let target_layer =
            match Self::resolve_layer_name_from_inner(&inner, layer_name, true) {
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
                is_native_terminal: false,
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
            state
                .windows
                .insert(toplevel.id(), ToplevelWindow::default());
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
            zwlr_foreign_toplevel_handle_v1::Event::State {
                state: handle_state,
            } => {
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
            state
                .windows
                .insert(toplevel.id(), ToplevelWindow::default());
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
            zcosmic_toplevel_handle_v1::Event::State {
                state: handle_state,
            } => {
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
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
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
        return Err(
            "No supported toplevel protocol (wlr-foreign-toplevel or cosmic-toplevel-info)".into(),
        );
    };

    println!("[Wayland] Using {:?} toplevel protocol", protocol);

    // Initial roundtrip to populate state
    queue.roundtrip(&mut state)?;

    println!("[Wayland] Listening for focus events...");

    let raw_fd = connection.as_fd().as_raw_fd();
    let async_fd = AsyncFd::new(RawFdWatcher::new(raw_fd))?;
    let mut shutdown_receiver = shutdown_handle.subscribe();

    apply_focus_for_env(
        Environment::Wayland,
        None,
        false,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    loop {
        if *shutdown_receiver.borrow() {
            return Ok(());
        }

        let dispatched = queue.dispatch_pending(&mut state)?;
        if dispatched > 0 {
            let win = state.get_active_window();
            let default_layer = kanata.default_layer_sync();
            if let Some(actions) = handle_focus_event(
                &handler,
                &status_broadcaster,
                &pause_broadcaster,
                &win,
                &kanata,
                &default_layer,
            )
            .await
            {
                execute_focus_actions(&kanata, actions).await;
            }
            continue;
        }

        connection.flush()?;
        let guard = match queue.prepare_read() {
            Some(guard) => guard,
            None => continue,
        };

        let mut readiness = tokio::select! {
            _ = shutdown_receiver.changed() => {
                return Ok(());
            }
            readiness = async_fd.readable() => readiness?,
        };

        let read_result = guard.read();
        readiness.clear_ready();

        match read_result {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                eprintln!("[Wayland] Read error: {}", error);
                return Err(error.into());
            }
        }

        let _ = queue.dispatch_pending(&mut state)?;
        let win = state.get_active_window();
        let default_layer = kanata.default_layer_sync();

        if let Some(actions) = handle_focus_event(
            &handler,
            &status_broadcaster,
            &pause_broadcaster,
            &win,
            &kanata,
            &default_layer,
        )
        .await
        {
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

        Ok(Self {
            connection,
            root,
            atoms,
        })
    }

    fn get_active_window_id(&self) -> Option<Window> {
        let prop_reply = self
            .connection
            .get_property(
                false,
                self.root,
                self.atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ == x11rb::NONE || prop_reply.value.len() != 4 {
            return None;
        }

        let arr: [u8; 4] = prop_reply.value.clone().try_into().ok()?;
        let winid = u32::from_le_bytes(arr);

        if winid == 0 { None } else { Some(winid) }
    }

    fn get_window_class(&self, window: Window) -> Option<String> {
        let reply = self
            .connection
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
        let prop_reply = self
            .connection
            .get_property(
                false,
                window,
                self.atoms._NET_WM_NAME,
                self.atoms.UTF8_STRING,
                0,
                u32::MAX,
            )
            .ok()?
            .reply()
            .ok()?;

        if prop_reply.type_ != x11rb::NONE {
            return String::from_utf8(prop_reply.value).ok();
        }

        // Fallback to WM_NAME (Latin-1)
        let prop_reply = self
            .connection
            .get_property(
                false,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                0,
                u32::MAX,
            )
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

        WindowInfo {
            class,
            title,
            is_native_terminal: false,
        }
    }
}

async fn run_x11(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new()?;

    println!("[X11] Connected to display");

    apply_focus_for_env(
        Environment::X11,
        None,
        false,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    println!("[X11] Listening for focus events...");

    let raw_fd = state.connection.stream().as_raw_fd();
    let async_fd = AsyncFd::new(RawFdWatcher::new(raw_fd))?;
    let mut shutdown_receiver = shutdown_handle.subscribe();

    // Event loop - wait for PropertyNotify events on _NET_ACTIVE_WINDOW
    loop {
        if *shutdown_receiver.borrow() {
            return Ok(());
        }

        while let Some(event) = state.connection.poll_for_event()? {
            match event {
                X11Event::PropertyNotify(e) if e.atom == state.atoms._NET_ACTIVE_WINDOW => {
                    let win = state.get_active_window();
                    let default_layer = kanata.default_layer_sync();

                    if let Some(actions) = handle_focus_event(
                        &handler,
                        &status_broadcaster,
                        &pause_broadcaster,
                        &win,
                        &kanata,
                        &default_layer,
                    )
                    .await
                    {
                        execute_focus_actions(&kanata, actions).await;
                    }
                }
                _ => {}
            }
        }

        let mut readiness = tokio::select! {
            _ = shutdown_receiver.changed() => {
                return Ok(());
            }
            readiness = async_fd.readable() => readiness?,
        };
        readiness.clear_ready();
    }
}

fn start_sni_indicator(
    control: SniControl,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    indicator_focus_only: Option<TrayFocusOnly>,
) -> Option<ksni::Handle<SniIndicator>> {
    println!("[SNI] Starting StatusNotifier indicator");
    let initial_status = status_broadcaster.snapshot();
    let mut settings = SniSettingsStore::new();
    let show_focus_only = resolve_sni_focus_only(indicator_focus_only, &mut settings);
    let (menu_refresh, mut menu_refresh_receiver) = MenuRefresh::new();
    let control_handle: Arc<dyn SniControlOps> = Arc::new(control);
    let indicator = SniIndicator {
        state: SniIndicatorState::new(initial_status, show_focus_only),
        control: control_handle,
        settings,
        menu_refresh,
    };
    let service = TrayService::new(indicator);
    let handle = service.handle();

    let pause_initial = pause_broadcaster.is_paused();
    handle.update(|state| state.set_paused(pause_initial));

    let status_handle = handle.clone();
    let mut status_receiver = status_broadcaster.subscribe();
    tokio::spawn(async move {
        loop {
            if status_receiver.changed().await.is_err() {
                break;
            }
            let snapshot = status_receiver.borrow().clone();
            status_handle.update(|state| state.update_status(snapshot));
        }
    });

    let pause_handle = handle.clone();
    let mut pause_receiver = pause_broadcaster.subscribe();
    tokio::spawn(async move {
        loop {
            if pause_receiver.changed().await.is_err() {
                break;
            }
            let paused = *pause_receiver.borrow();
            pause_handle.update(|state| state.set_paused(paused));
        }
    });

    let menu_handle = handle.clone();
    tokio::spawn(async move {
        loop {
            if menu_refresh_receiver.changed().await.is_err() {
                break;
            }
            menu_handle.update(|state| state.state.bump_menu_revision());
        }
    });

    thread::spawn(move || match service.run() {
        Ok(()) => println!("[SNI] Indicator stopped"),
        Err(error) => eprintln!("[SNI] Failed to run indicator: {}", error),
    });

    Some(handle)
}

struct SniGuard {
    handle: Option<ksni::Handle<SniIndicator>>,
}

impl SniGuard {
    fn new(handle: Option<ksni::Handle<SniIndicator>>) -> Self {
        Self { handle }
    }
}

impl Drop for SniGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            println!("[SNI] Shutting down indicator");
            handle.shutdown();
        }
    }
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if dirs.iter().any(|existing| existing == &path) {
        return;
    }
    dirs.push(path);
}

fn nixos_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    push_unique_dir(&mut dirs, PathBuf::from(NIXOS_SYSTEM_PROFILE));
    push_unique_dir(&mut dirs, PathBuf::from(NIXOS_DEFAULT_PROFILE));

    if let Ok(user) = env::var("USER") {
        if !user.is_empty() {
            push_unique_dir(
                &mut dirs,
                PathBuf::from(NIXOS_PER_USER_PROFILE_PREFIX)
                    .join(&user)
                    .join("share"),
            );
            push_unique_dir(
                &mut dirs,
                PathBuf::from(NIXOS_PER_USER_PROFILE_ALT_PREFIX)
                    .join(&user)
                    .join("profile")
                    .join("share"),
            );
        }
    }

    dirs
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        if !xdg_data_home.is_empty() {
            push_unique_dir(&mut dirs, PathBuf::from(xdg_data_home));
        }
    } else if let Ok(home) = env::var("HOME") {
        if !home.is_empty() {
            push_unique_dir(&mut dirs, PathBuf::from(home).join(".local/share"));
        }
    }

    let data_dirs =
        env::var("XDG_DATA_DIRS").unwrap_or_else(|_| XDG_DATA_DIRS_FALLBACK.to_string());
    for entry in data_dirs.split(':') {
        if entry.is_empty() {
            continue;
        }
        push_unique_dir(&mut dirs, PathBuf::from(entry));
    }

    for nix_dir in nixos_data_dirs() {
        push_unique_dir(&mut dirs, nix_dir);
    }

    dirs
}

fn push_schema_dir_if_exists(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if path.join(GSETTINGS_COMPILED_FILENAME).exists() {
        push_unique_dir(dirs, path);
    }
}

fn find_gsettings_schema_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for data_dir in xdg_data_dirs() {
        push_schema_dir_if_exists(&mut dirs, data_dir.join("glib-2.0").join("schemas"));
        push_schema_dir_if_exists(
            &mut dirs,
            data_dir
                .join("gnome-shell")
                .join("extensions")
                .join(GNOME_EXTENSION_UUID)
                .join("schemas"),
        );
    }

    push_schema_dir_if_exists(&mut dirs, get_gnome_extension_fs_path().join("schemas"));

    dirs
}

fn gsettings_command(schema_dir: Option<&Path>) -> Command {
    let mut command = Command::new("gsettings");
    if let Some(dir) = schema_dir {
        command.arg("--schemadir").arg(dir);
    }
    command
}

fn gsettings_get_bool(schema_dir: Option<&Path>, schema: &str, key: &str) -> Result<bool, String> {
    let output = gsettings_command(schema_dir)
        .args(["get", schema, key])
        .output()
        .map_err(|error| format!("gsettings get failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gsettings get failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(format!("unexpected gsettings output: {}", value)),
    }
}

fn gsettings_set_bool(
    schema_dir: Option<&Path>,
    schema: &str,
    key: &str,
    value: bool,
) -> Result<(), String> {
    let value_str = if value { "true" } else { "false" };
    let output = gsettings_command(schema_dir)
        .args(["set", schema, key, value_str])
        .output()
        .map_err(|error| format!("gsettings set failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gsettings set failed: {}", stderr.trim()));
    }

    Ok(())
}

fn is_gsettings_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("no such file or directory") || lower.contains("not found")
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
const EMBEDDED_DBUS_JS: &str = include_str!(gnome_ext_file!("dbus.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_FOCUS_JS: &str = include_str!(gnome_ext_file!("focus.js"));
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
        && path.join("dbus.js").exists()
        && path.join("focus.js").exists()
        && path.join(GNOME_EXTENSION_SCHEMA_FILE).exists()
        && path.join(GNOME_EXTENSION_SCHEMA_COMPILED).exists()
}

#[cfg(feature = "embed-gnome-extension")]
fn compile_gnome_schemas(dir: &Path) -> std::io::Result<()> {
    let schema_dir = dir.join("schemas");
    let output = Command::new("glib-compile-schemas")
        .arg(&schema_dir)
        .output()?;
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
    fs::write(dir.join("dbus.js"), EMBEDDED_DBUS_JS)?;
    fs::write(dir.join("focus.js"), EMBEDDED_FOCUS_JS)?;
    let schema_dir = dir.join("schemas");
    fs::create_dir_all(&schema_dir)?;
    fs::write(
        dir.join(GNOME_EXTENSION_SCHEMA_FILE),
        EMBEDDED_GSETTINGS_SCHEMA,
    )?;
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
fn parse_gnome_extension_state(
    body: &HashMap<String, zbus::zvariant::OwnedValue>,
) -> GnomeExtensionStatus {
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
            eprintln!(
                "[GNOME] D-Bus probe: failed to connect to session bus: {}",
                e
            );
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

    GnomeExtensionStatus {
        installed,
        enabled,
        active: false,
        state: None,
        method: GnomeDetectionMethod::Cli,
    }
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
            GNOME_EXTENSION_SRC_PATH, GNOME_EXTENSION_UUID, GNOME_EXTENSION_UUID
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
                print_gnome_extension_install_instructions(&format!("Auto-install failed: {}", e));
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
            "Extension files not found and embedded extension is disabled in this build."
                .to_string()
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
            print_gnome_extension_install_instructions(
                "Auto-install was disabled (--no-install-gnome-extension).",
            );
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
            if status.installed {
                "installed"
            } else {
                "not installed"
            },
            if status.enabled {
                "enabled"
            } else {
                "not enabled"
            },
            method_str,
            state_info,
            if !matches!(status.state, Some(2) | Some(4)) {
                " - waiting for GNOME Shell..."
            } else {
                ""
            }
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
    env: Environment,
    focus_query_connection: Connection,
    is_kde6: bool,
}

#[zbus::interface(name = "com.github.kanata.Switcher")]
impl DbusWindowFocusService {
    async fn window_focus(&self, window_class: &str, window_title: &str) {
        let win = WindowInfo {
            class: window_class.to_string(),
            title: window_title.to_string(),
            is_native_terminal: false,
        };

        if self.pause_broadcaster.is_paused() {
            return;
        }

        let default_layer = self
            .runtime_handle
            .block_on(async { self.kanata.default_layer().await })
            .unwrap_or_default();

        let actions = self.runtime_handle.block_on(async {
            update_status_for_focus(
                &self.handler,
                &self.status_broadcaster,
                &win,
                &self.kanata,
                &default_layer,
            )
            .await
        });

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
    async fn paused_changed(signal_emitter: &SignalEmitter<'_>, paused: bool) -> zbus::Result<()>;

    async fn restart(&self) {
        println!("[Restart] Restart requested via DBus");
        self.restart_handle.request();
    }

    async fn pause(&self) {
        pause_daemon(
            &self.pause_broadcaster,
            &self.handler,
            &self.status_broadcaster,
            &self.kanata,
            &self.runtime_handle,
            "via DBus",
        );
    }

    async fn unpause(&self) {
        unpause_daemon(
            self.env,
            Some(self.focus_query_connection.clone()),
            self.is_kde6,
            &self.pause_broadcaster,
            &self.handler,
            &self.status_broadcaster,
            &self.kanata,
            &self.runtime_handle,
            "via DBus",
        );
    }
}

async fn register_dbus_service(
    connection: &Connection,
    focus_query_connection: Connection,
    env: Environment,
    is_kde6: bool,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = DbusWindowFocusService {
        kanata,
        handler,
        runtime_handle: tokio::runtime::Handle::current(),
        status_broadcaster: status_broadcaster.clone(),
        restart_handle,
        pause_broadcaster: pause_broadcaster.clone(),
        env,
        focus_query_connection,
        is_kde6,
    };

    connection
        .object_server()
        .at("/com/github/kanata/Switcher", service)
        .await?;

    connection
        .request_name("com.github.kanata.Switcher")
        .await?;

    let mut receiver = status_broadcaster.subscribe();
    let signal_emitter =
        SignalEmitter::new(connection, "/com/github/kanata/Switcher")?.into_owned();
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
                let virtual_keys: Vec<&str> =
                    current.virtual_keys.iter().map(|vk| vk.as_str()).collect();
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
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    let focus_query_connection = Connection::session().await?;
    register_dbus_service(
        &connection,
        focus_query_connection.clone(),
        Environment::Gnome,
        false,
        kanata.clone(),
        handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
    )
    .await?;

    apply_focus_for_env(
        Environment::Gnome,
        Some(&focus_query_connection),
        false,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    println!("[GNOME] Listening for focus events from extension...");
    let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
    Ok(outcome)
}

// === KDE Backend ===

#[derive(Debug)]
struct KwinScriptGuard {
    connection: Connection,
    runtime_handle: tokio::runtime::Handle,
    script_path: String,
    script_obj_path: OwnedObjectPath,
    script_interface: String,
}

impl KwinScriptGuard {
    fn new(
        connection: Connection,
        runtime_handle: tokio::runtime::Handle,
        script_path: String,
        script_obj_path: OwnedObjectPath,
        script_interface: &str,
    ) -> Self {
        Self {
            connection,
            runtime_handle,
            script_path,
            script_obj_path,
            script_interface: script_interface.to_string(),
        }
    }
}

impl Drop for KwinScriptGuard {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let runtime_handle = self.runtime_handle.clone();
        let script_path = self.script_path.clone();
        let script_obj_path = self.script_obj_path.clone();
        let script_interface = self.script_interface.clone();

        let cleanup = async move {
            let stop_result = connection
                .call_method(
                    Some("org.kde.KWin"),
                    script_obj_path.clone(),
                    Some(script_interface.as_str()),
                    "stop",
                    &(),
                )
                .await;
            if let Err(error) = stop_result {
                panic!("[KDE] Failed to stop KWin script: {}", error);
            }

            let unload_result = connection
                .call_method(
                    Some("org.kde.KWin"),
                    "/Scripting",
                    Some("org.kde.kwin.Scripting"),
                    "unloadScript",
                    &(&script_path,),
                )
                .await;
            if let Err(error) = unload_result {
                panic!("[KDE] Failed to unload KWin script: {}", error);
            }
        };

        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| {
                runtime_handle.block_on(cleanup);
            });
        } else {
            runtime_handle.block_on(cleanup);
        }

        if let Err(error) = fs::remove_file(&self.script_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                panic!("[KDE] Failed to remove KWin script file: {}", error);
            }
        }
    }
}

async fn run_kde(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    let focus_query_connection = Connection::session().await?;
    let runtime_handle = tokio::runtime::Handle::current();
    let is_kde6 = env::var("KDE_SESSION_VERSION")
        .map(|v| v == "6")
        .unwrap_or(false);
    register_dbus_service(
        &connection,
        focus_query_connection.clone(),
        Environment::Kde,
        is_kde6,
        kanata.clone(),
        handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
    )
    .await?;

    apply_focus_for_env(
        Environment::Kde,
        Some(&focus_query_connection),
        is_kde6,
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &kanata,
    )
    .await?;

    // Inject KWin script (DBus service is ready to receive calls)
    let api = if is_kde6 {
        "windowActivated"
    } else {
        "clientActivated"
    };
    let active_window = if is_kde6 {
        "activeWindow"
    } else {
        "activeClient"
    };
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

    let script_obj_path: OwnedObjectPath = script_obj_path_str.as_str().try_into()?;

    let _kwin_script_guard = KwinScriptGuard::new(
        connection.clone(),
        runtime_handle.clone(),
        script_path.clone(),
        script_obj_path.clone(),
        script_interface,
    );

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

    let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
    Ok(outcome)
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
    if args.install_autostart {
        install_autostart_desktop(&matches, &args)?;
        return Ok(RunOutcome::Exit);
    }
    if args.uninstall_autostart {
        uninstall_autostart_desktop()?;
        return Ok(RunOutcome::Exit);
    }
    if let Some(command) = resolve_control_command(&args) {
        send_control_command(command).await?;
        return Ok(RunOutcome::Exit);
    }

    let install_gnome_extension = resolve_install_gnome_extension(&matches);

    let env = detect_environment();
    println!("[Init] Detected environment: {}", env.as_str());

    if env == Environment::Gnome {
        setup_gnome_extension(install_gnome_extension);
    }

    let config = load_config(args.config.as_deref());
    if config.rules.is_empty() && config.native_terminal_rule.is_none() {
        eprintln!("[Config] Error: No rules found in config file");
        eprintln!();
        eprintln!("Example config (~/.config/kanata/kanata-switcher.json):");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    let quiet_focus = args.quiet || args.quiet_focus;
    let status_broadcaster = StatusBroadcaster::new();
    let restart_handle = RestartHandle::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let shutdown_handle = ShutdownHandle::new();
    let runtime_handle = tokio::runtime::Handle::current();
    let kanata = KanataClient::new(
        &args.host,
        args.port,
        config.default_layer,
        args.quiet,
        status_broadcaster.clone(),
    );
    kanata.connect_with_retry().await;

    let focus_handler = if matches!(env, Environment::Unknown) {
        None
    } else {
        Some(Arc::new(Mutex::new(FocusHandler::new(
            config.rules.clone(),
            config.native_terminal_rule.clone(),
            quiet_focus,
        ))))
    };

    if let Some(handler) = focus_handler.clone() {
        let session_connection = if matches!(env, Environment::Gnome | Environment::Kde) {
            Some(Connection::session().await?)
        } else {
            None
        };
        let is_kde6 = env::var("KDE_SESSION_VERSION")
            .map(|v| v == "6")
            .unwrap_or(false);
        start_logind_session_monitor_best_effort(
            env,
            session_connection,
            is_kde6,
            handler,
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
            kanata.clone(),
            start_logind_session_monitor,
        )
        .await;
    }

    // Create shutdown guard - will switch to default layer when dropped
    let _shutdown_guard = ShutdownGuard::new(kanata.clone());

    // Set up signal handlers
    let shutdown_handle_for_signal = shutdown_handle.clone();
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

        shutdown_handle_for_signal.request();
    });

    let enable_indicator = !args.no_indicator && env != Environment::Gnome;
    if args.no_indicator && env != Environment::Gnome {
        println!("[SNI] Indicator disabled via --no-indicator");
    }

    let sni_control = if enable_indicator {
        match env {
            Environment::Kde => match Connection::session().await {
                Ok(connection) => Some(SniControl::Dbus(SniDbusControl {
                    runtime_handle: runtime_handle.clone(),
                    connection,
                    restart_handle: restart_handle.clone(),
                })),
                Err(error) => {
                    eprintln!("[SNI] Failed to connect to session bus: {}", error);
                    None
                }
            },
            Environment::Wayland | Environment::X11 => {
                let handler = focus_handler
                    .clone()
                    .expect("Focus handler missing for non-GNOME backend");
                Some(SniControl::Local(SniLocalControl {
                    runtime_handle: runtime_handle.clone(),
                    kanata: kanata.clone(),
                    handler,
                    status_broadcaster: status_broadcaster.clone(),
                    pause_broadcaster: pause_broadcaster.clone(),
                    restart_handle: restart_handle.clone(),
                    env,
                    connection: None,
                    is_kde6: false,
                }))
            }
            _ => None,
        }
    } else {
        None
    };

    let sni_handle = sni_control.and_then(|control| {
        start_sni_indicator(
            control,
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
            args.indicator_focus_only,
        )
    });
    let _sni_guard = SniGuard::new(sni_handle);

    match env {
        Environment::Gnome => {
            let handler = focus_handler.expect("Focus handler missing for GNOME backend");
            return run_gnome(
                kanata,
                handler,
                status_broadcaster,
                restart_handle,
                pause_broadcaster,
                shutdown_handle,
            )
            .await;
        }
        Environment::Kde => {
            let handler = focus_handler.expect("Focus handler missing for KDE backend");
            return run_kde(
                kanata,
                handler,
                status_broadcaster,
                restart_handle,
                pause_broadcaster,
                shutdown_handle,
            )
            .await;
        }
        Environment::Wayland => {
            let handler = focus_handler.expect("Focus handler missing for Wayland backend");
            run_wayland(
                kanata,
                handler,
                status_broadcaster,
                pause_broadcaster,
                shutdown_handle,
            )
            .await?;
        }
        Environment::X11 => {
            let handler = focus_handler.expect("Focus handler missing for X11 backend");
            run_x11(
                kanata,
                handler,
                status_broadcaster,
                pause_broadcaster,
                shutdown_handle,
            )
            .await?;
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
