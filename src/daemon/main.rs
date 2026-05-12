use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use futures_util::StreamExt;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon as SniIcon, MenuItem, Status as SniStatus, ToolTip, Tray, TrayService};
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::fd::AsFd;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::io::unix::AsyncFd;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot, watch};
use uuid::Uuid;
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
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

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

mod constants;
mod errors;
mod environ;
mod dbus_naming;
mod config;
mod focus;
mod args;
mod autostart;
mod broadcasters;
mod kanata;

use constants::*;
use errors::DynError;
use environ::*;
use dbus_naming::*;
use config::*;
use focus::*;
use args::*;
use autostart::*;
use broadcasters::*;
use kanata::*;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::{constants::*, errors::*, environ::*, dbus_naming::*, config::*, focus::*, args::*, autostart::*, broadcasters::*, kanata::*};


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

/// Per-instance control dispatch mode. `Unicast` targets a single daemon bus
/// name; `Broadcast` enumerates all daemons in the `instances.*` namespace.
enum ControlDispatch {
    Unicast { bus_name: String },
    Broadcast,
}

async fn send_control_command(
    command: ControlCommand,
    dispatch: ControlDispatch,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    match dispatch {
        ControlDispatch::Unicast { bus_name } => {
            send_control_command_with_connection(&connection, &bus_name, command).await?;
            println!("[Control] {}: {} ok", bus_name, command.label());
            Ok(())
        }
        ControlDispatch::Broadcast => {
            let report = send_control_command_broadcast(&connection, command).await?;
            for entry in &report.results {
                match &entry.outcome {
                    Ok(()) => println!("[Control] {}: {} ok", entry.bus_name, command.label()),
                    Err(error) => eprintln!(
                        "[Control] {}: {} failed: {}",
                        entry.bus_name,
                        command.label(),
                        error
                    ),
                }
            }
            if report.results.iter().all(|entry| entry.outcome.is_err()) {
                return Err(format!(
                    "All daemons failed to respond to {}",
                    command.label()
                )
                .into());
            }
            Ok(())
        }
    }
}

/// Send a control command to a specific daemon by bus name. Used by both the
/// unicast control CLI path and the broadcast fan-out (per-target).
async fn send_control_command_with_connection(
    connection: &Connection,
    bus_name: &str,
    command: ControlCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    connection
        .call_method(
            Some(bus_name),
            DBUS_PATH,
            Some(DBUS_INTERFACE),
            command.dbus_method(),
            &(),
        )
        .await?;
    Ok(())
}

#[derive(Debug)]
struct BroadcastEntryReport {
    bus_name: String,
    outcome: Result<(), Box<dyn std::error::Error + Send + Sync>>,
}

#[derive(Debug)]
struct BroadcastReport {
    results: Vec<BroadcastEntryReport>,
}

/// Enumerate every well-known bus name in the `instances.*` subtree.
async fn enumerate_daemon_names(
    connection: &Connection,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let names = dbus.list_names().await?;
    let mut filtered: Vec<String> = names
        .into_iter()
        .map(|name| name.as_str().to_string())
        .filter(|name| is_daemon_bus_name(name))
        .collect();
    filtered.sort();
    filtered.dedup();
    Ok(filtered)
}

const BROADCAST_PER_CALL_TIMEOUT: Duration = Duration::from_secs(2);

async fn send_control_command_broadcast(
    connection: &Connection,
    command: ControlCommand,
) -> Result<BroadcastReport, Box<dyn std::error::Error + Send + Sync>> {
    let names = enumerate_daemon_names(connection).await?;
    if names.is_empty() {
        return Err(
            "No daemons running (no owners under com.github.kanata.Switcher.instances.*)".into(),
        );
    }
    // Dispatch in parallel so one hanging daemon doesn't extend total broadcast
    // time by `N * per_call_timeout`. Per-call `tokio::time::timeout` still
    // bounds individual call time; the futures share the same connection
    // (cheap to clone — `zbus::Connection` is internally `Arc`-backed).
    let futures = names.iter().cloned().map(|name| {
        let connection = connection.clone();
        async move {
            let outcome = tokio::time::timeout(
                BROADCAST_PER_CALL_TIMEOUT,
                send_control_command_with_connection(&connection, &name, command),
            )
            .await;
            let outcome: Result<(), Box<dyn std::error::Error + Send + Sync>> = match outcome {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(format!(
                    "timed out after {}ms",
                    BROADCAST_PER_CALL_TIMEOUT.as_millis()
                )
                .into()),
            };
            BroadcastEntryReport {
                bus_name: name,
                outcome,
            }
        }
    });
    let results = futures_util::future::join_all(futures).await;
    Ok(BroadcastReport { results })
}

// === SNI Indicator ===

const SNI_DEFAULT_SHOW_FOCUS_ONLY: bool = true;
const SNI_FONT_WEIGHT: FontWeight = FontWeight::Regular;
const SNI_RASTER_HEIGHT: RasterHeight = RasterHeight::Size32;
const SNI_GLYPH_WIDTH: usize = get_raster_width(SNI_FONT_WEIGHT, SNI_RASTER_HEIGHT);
const SNI_GLYPH_HEIGHT: usize = SNI_RASTER_HEIGHT.val();
const SNI_GLYPH_GAP: usize = 4;
const SNI_ICON_HEIGHT: usize = SNI_GLYPH_HEIGHT;
const SNI_COLOR_LAYER: [u8; 4] = [255, 255, 255, 255];
const SNI_COLOR_VK: [u8; 4] = [255, 0, 255, 255];
const SNI_MAX_VK_COUNT_DIGIT: usize = 9;
const SNI_MIN_MULTI_VK_COUNT: usize = 2;
const SNI_INDICATOR_ID: &str = "kanata-switcher";

trait DconfBackend: Send + Sync {
    fn get_bool(&self, key: &str) -> Result<bool, String>;
    fn set_bool(&self, key: &str, value: bool) -> Result<(), String>;
}

struct ShellDconfBackend;

impl DconfBackend for ShellDconfBackend {
    fn get_bool(&self, key: &str) -> Result<bool, String> {
        dconf_get_bool(key)
    }

    fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
        dconf_set_bool(key, value)
    }
}

struct SniSettingsStore {
    available: bool,
    backend: Box<dyn DconfBackend>,
}

impl SniSettingsStore {
    fn new() -> Self {
        Self {
            available: true,
            backend: Box::new(ShellDconfBackend),
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Box<dyn DconfBackend>) -> Self {
        Self {
            available: true,
            backend,
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            available: false,
            backend: Box::new(ShellDconfBackend),
        }
    }

    fn read_focus_only(&mut self) -> Option<bool> {
        if !self.available {
            return None;
        }
        match self.backend.get_bool(DCONF_FOCUS_ONLY_KEY) {
            Ok(value) => Some(value),
            Err(error) => {
                if is_dconf_unavailable(&error) {
                    self.available = false;
                }
                eprintln!("[SNI] dconf read failed: {}", error);
                None
            }
        }
    }

    fn write_focus_only(&mut self, value: bool) {
        if !self.available {
            return;
        }
        if let Err(error) = self.backend.set_bool(DCONF_FOCUS_ONLY_KEY, value) {
            if is_dconf_unavailable(&error) {
                self.available = false;
            }
            eprintln!("[SNI] dconf write failed: {}", error);
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
        (Self { sender, version: 0 }, receiver)
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
struct UnpauseContext {
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
}

#[derive(Clone)]
struct SniLocalControl {
    runtime_handle: tokio::runtime::Handle,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    unpause_context: UnpauseContext,
}

#[derive(Clone)]
struct SniDbusControl {
    runtime_handle: tokio::runtime::Handle,
    connection: Connection,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    /// Per-instance daemon bus name to target with DBus control calls.
    daemon_bus_name: String,
}

#[derive(Clone)]
enum SniControl {
    Local(SniLocalControl),
    Dbus(SniDbusControl),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SniControlMode {
    Local,
    Dbus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SniRuntimeTransitionPlan {
    Keep,
    Stop,
    Start(SniControlMode),
    Restart(SniControlMode),
}

fn sni_control_mode_for_environment(env: Environment) -> Option<SniControlMode> {
    match env {
        Environment::Wayland | Environment::X11 => Some(SniControlMode::Local),
        Environment::Kde => Some(SniControlMode::Dbus),
        Environment::Gnome | Environment::LinuxConsoleWithLogind | Environment::Unknown => None,
    }
}

fn local_sni_unpause_context(env: Environment) -> UnpauseContext {
    match env {
        Environment::Wayland | Environment::X11 => UnpauseContext {
            env,
            connection: None,
            is_kde6: false,
        },
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => {
            panic!(
                "[SNI] Local control created for unsupported environment: {:?}",
                env
            )
        }
    }
}

fn plan_sni_runtime_transition(
    active_mode: Option<SniControlMode>,
    active_env: Option<Environment>,
    desired_env: Environment,
) -> SniRuntimeTransitionPlan {
    let desired_mode = sni_control_mode_for_environment(desired_env);
    match (active_mode, desired_mode) {
        (None, None) => SniRuntimeTransitionPlan::Keep,
        (Some(_), None) => SniRuntimeTransitionPlan::Stop,
        (None, Some(mode)) => SniRuntimeTransitionPlan::Start(mode),
        (Some(current_mode), Some(next_mode)) => {
            if current_mode != next_mode || active_env != Some(desired_env) {
                SniRuntimeTransitionPlan::Restart(next_mode)
            } else {
                SniRuntimeTransitionPlan::Keep
            }
        }
    }
}

const SNI_RUNTIME_RETRY_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SniRuntimeWakeReason {
    EnvironmentChanged,
    Retry,
    ChannelClosed,
}

async fn wait_for_sni_runtime_wake_with_delay(
    env_receiver: &mut watch::Receiver<Environment>,
    should_retry_start: bool,
    retry_delay: Duration,
) -> SniRuntimeWakeReason {
    if !should_retry_start {
        return match env_receiver.changed().await {
            Ok(_) => SniRuntimeWakeReason::EnvironmentChanged,
            Err(_) => SniRuntimeWakeReason::ChannelClosed,
        };
    }

    tokio::select! {
        changed = env_receiver.changed() => {
            match changed {
                Ok(_) => SniRuntimeWakeReason::EnvironmentChanged,
                Err(_) => SniRuntimeWakeReason::ChannelClosed,
            }
        }
        _ = tokio::time::sleep(retry_delay) => SniRuntimeWakeReason::Retry,
    }
}

trait SniControlOps: Send + Sync {
    fn restart(&self);
    fn pause(&self);
    fn unpause(&self);
    fn quit(&self);
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
                        &control.daemon_bus_name,
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
                        &control.daemon_bus_name,
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
                let context = control.unpause_context.clone();
                unpause_daemon(
                    context.env,
                    context.connection,
                    context.is_kde6,
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
                        &control.daemon_bus_name,
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

    fn quit(&self) {
        println!("[SNI] Quit requested");
        match self {
            SniControl::Local(control) => {
                control.shutdown_handle.request();
            }
            SniControl::Dbus(control) => {
                control.shutdown_handle.request();
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

    fn request_quit(&self) {
        self.control.quit();
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
            return format!("{}+", SNI_MAX_VK_COUNT_DIGIT);
        }
        count.to_string()
    }

    fn glyph_for_char(ch: char) -> RasterizedChar {
        get_raster(ch, SNI_FONT_WEIGHT, SNI_RASTER_HEIGHT)
            .or_else(|| get_raster('?', SNI_FONT_WEIGHT, SNI_RASTER_HEIGHT))
            .expect("SNI glyph lookup failed")
    }

    fn draw_glyph(
        buffer: &mut [u8],
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        glyph: &RasterizedChar,
        color: [u8; 4],
    ) {
        for (row_index, row) in glyph.raster().iter().enumerate() {
            let dest_y = y + row_index;
            if dest_y >= height {
                continue;
            }
            for (col_index, intensity) in row.iter().enumerate() {
                let dest_x = x + col_index;
                if dest_x >= width {
                    continue;
                }
                if *intensity == 0 {
                    continue;
                }
                let offset = (dest_y * width + dest_x) * 4;
                let alpha = (u16::from(color[0]) * u16::from(*intensity) / 255) as u8;
                let red = (u16::from(color[1]) * u16::from(*intensity) / 255) as u8;
                let green = (u16::from(color[2]) * u16::from(*intensity) / 255) as u8;
                let blue = (u16::from(color[3]) * u16::from(*intensity) / 255) as u8;
                buffer[offset] = alpha;
                buffer[offset + 1] = red;
                buffer[offset + 2] = green;
                buffer[offset + 3] = blue;
            }
        }
    }

    fn text_width(text: &str) -> usize {
        SNI_GLYPH_WIDTH * text.chars().count()
    }

    fn draw_text(
        buffer: &mut [u8],
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        text: &str,
        color: [u8; 4],
    ) -> usize {
        let mut cursor_x = x;
        for ch in text.chars() {
            let glyph = Self::glyph_for_char(ch);
            Self::draw_glyph(buffer, width, height, cursor_x, y, &glyph, color);
            cursor_x += glyph.width();
        }
        cursor_x - x
    }

    fn render_icon(layer_text: &str, vk_text: &str) -> SniIcon {
        let layer_width = Self::text_width(layer_text);
        let vk_width = Self::text_width(vk_text);
        let gap = if vk_text.is_empty() { 0 } else { SNI_GLYPH_GAP };
        let icon_width = layer_width + gap + vk_width;
        let mut buffer = vec![0u8; icon_width * SNI_ICON_HEIGHT * 4];
        let glyph_y = (SNI_ICON_HEIGHT - SNI_GLYPH_HEIGHT) / 2;
        let layer_x = 0;
        let vk_x = layer_x + layer_width + gap;

        Self::draw_text(
            &mut buffer,
            icon_width,
            SNI_ICON_HEIGHT,
            layer_x,
            glyph_y,
            layer_text,
            SNI_COLOR_LAYER,
        );
        if !vk_text.is_empty() {
            Self::draw_text(
                &mut buffer,
                icon_width,
                SNI_ICON_HEIGHT,
                vk_x,
                glyph_y,
                vk_text,
                SNI_COLOR_VK,
            );
        }

        SniIcon {
            width: icon_width as i32,
            height: SNI_ICON_HEIGHT as i32,
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

    fn title_text(&self) -> String {
        "Kanata Switcher".to_string()
    }
}

impl Tray for SniIndicator {
    fn id(&self) -> String {
        SNI_INDICATOR_ID.to_string()
    }

    fn title(&self) -> String {
        self.title_text()
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
            title: self.title_text(),
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
            MenuItem::Standard(StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|this| {
                    this.request_quit();
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

    // Filter out invalid VKs before updating indicator
    let known_vks = kanata.known_virtual_keys().await;
    let valid_virtual_keys = KanataClient::filter_valid_virtual_keys(&known_vks, virtual_keys);
    status_broadcaster.update_virtual_keys(valid_virtual_keys);
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

fn resolve_wayland_socket_path(
    wayland_display: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let socket_name = PathBuf::from(wayland_display);
    if socket_name.is_absolute() {
        return Ok(socket_name);
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("XDG_RUNTIME_DIR is not set")?;
    if !runtime_dir.is_absolute() {
        return Err("XDG_RUNTIME_DIR must be an absolute path".into());
    }

    Ok(runtime_dir.join(socket_name))
}

fn connect_wayland_with_display_override(
    wayland_display_override: Option<&str>,
) -> Result<WaylandConnection, Box<dyn std::error::Error + Send + Sync>> {
    match wayland_display_override {
        Some(display) => {
            let socket_path = resolve_wayland_socket_path(display)?;
            let socket = UnixStream::connect(socket_path)?;
            Ok(WaylandConnection::from_socket(socket)?)
        }
        None => Ok(WaylandConnection::connect_to_env()?),
    }
}

fn query_wayland_active_window(
    wayland_display_override: Option<&str>,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(test)]
    {
        WAYLAND_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    }
    let connection = connect_wayland_with_display_override(wayland_display_override)?;
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

fn query_x11_active_window(
    x11_display_override: Option<&str>,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new(x11_display_override)?;
    Ok(state.get_active_window())
}

static KDE_QUERY_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static WAYLAND_QUERY_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn kwin_query_script_path(query_id: u64) -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!(
        "/tmp/kanata-switcher-kwin-query-{}-{}-{}.js",
        uid, query_id, request_id
    )
}

fn kwin_query_probe_script_path(probe_id: u64) -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!(
        "/tmp/kanata-switcher-kwin-query-probe-{}-{}-{}.js",
        uid, probe_id, request_id
    )
}

fn kwin_runtime_script_path() -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!("/tmp/kanata-switcher-kwin-{}-{}.js", uid, request_id)
}

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

    let script_path = kwin_query_script_path(query_id);
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
        Environment::Wayland => {
            let display_override = resolve_display_override_for_environment(env, "Focus").await;
            tokio::task::block_in_place(move || {
                query_wayland_active_window(display_override.as_deref())
            })
        }
        Environment::X11 => {
            let display_override = resolve_display_override_for_environment(env, "Focus").await;
            tokio::task::block_in_place(move || {
                query_x11_active_window(display_override.as_deref())
            })
        }
        Environment::LinuxConsoleWithLogind => Ok(native_terminal_window()),
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
async fn resolve_logind_session_path(
    connection: &Connection,
) -> Result<OwnedObjectPath, LogindSessionPathResolutionError> {
    let manager = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await
    .map_err(LogindSessionPathResolutionError::fatal)?;

    if let Ok(session_id) = env::var("XDG_SESSION_ID") {
        println!("[Logind] Using XDG_SESSION_ID={}", session_id);
        let reply = manager
            .call_method("GetSession", &(session_id))
            .await
            .map_err(LogindSessionPathResolutionError::fatal)?;
        let path = decode_logind_object_path_reply(&reply, "GetSession")
            .map_err(LogindSessionPathResolutionError::fatal)?;
        println!("[Logind] Using session path: {}", path.as_str());
        return Ok(path);
    }
    println!("[Logind] XDG_SESSION_ID not set; resolving session via logind");

    let pid = std::process::id();
    match manager.call_method("GetSessionByPID", &(pid)).await {
        Ok(reply) => {
            let path = decode_logind_object_path_reply(&reply, "GetSessionByPID")
                .map_err(LogindSessionPathResolutionError::fatal)?;
            println!("[Logind] Using session path: {}", path.as_str());
            Ok(path)
        }
        Err(error) => {
            if is_logind_no_session_error(&error) {
                return resolve_logind_display_session_path(&manager, connection, pid).await;
            }
            Err(LogindSessionPathResolutionError::fatal(error))
        }
    }
}

fn is_logind_no_session_error(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => name.as_ref() == LOGIND_ERROR_NO_SESSION_FOR_PID,
        _ => false,
    }
}

enum LogindSessionPathResolutionError {
    DisplayNotReady,
    Fatal(Box<dyn std::error::Error + Send + Sync>),
}

impl LogindSessionPathResolutionError {
    fn fatal<E>(error: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self::Fatal(error.into())
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
        Value::Structure(structure) => parse_logind_object_path_from_structure(structure),
        Value::Value(inner) => logind_object_path_from_value(inner),
        _ => None,
    }
}

async fn resolve_logind_display_session_path(
    manager: &zbus::Proxy<'_>,
    connection: &Connection,
    pid: u32,
) -> Result<OwnedObjectPath, LogindSessionPathResolutionError> {
    let user_reply = manager
        .call_method("GetUserByPID", &(pid))
        .await
        .map_err(LogindSessionPathResolutionError::fatal)?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")
        .map_err(LogindSessionPathResolutionError::fatal)?;
    let user_proxy = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await
    .map_err(LogindSessionPathResolutionError::fatal)?;
    let display = parse_logind_object_path(
        user_proxy
            .get_property::<OwnedValue>("Display")
            .await
            .map_err(LogindSessionPathResolutionError::fatal)?,
        "User.Display",
    )
    .map_err(LogindSessionPathResolutionError::fatal)?;
    if is_logind_empty_object_path(&display) {
        return Err(LogindSessionPathResolutionError::DisplayNotReady);
    }
    println!("[Logind] Using display session path: {}", display.as_str());
    Ok(display)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LogindDisplayPathChange {
    Unchanged,
    Empty,
    Path(OwnedObjectPath),
}

fn decode_logind_display_path_change(
    value: Option<&Value<'_>>,
) -> Result<LogindDisplayPathChange, String> {
    let Some(value) = value else {
        return Ok(LogindDisplayPathChange::Unchanged);
    };
    let path = logind_object_path_from_value(value).ok_or_else(|| {
        "[Lifecycle] Failed to parse logind User.Display property change".to_string()
    })?;
    if is_logind_empty_object_path(&path) {
        return Ok(LogindDisplayPathChange::Empty);
    }
    Ok(LogindDisplayPathChange::Path(path))
}

fn snapshot_no_session() -> LifecycleSnapshot {
    LifecycleSnapshot {
        active: false,
        session_type: String::new(),
        session_kind: SessionKind::NoSession,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LogindDisplayChangeAction {
    Ignore,
    DetachSessionMonitor,
    EmitNoSessionAndDetach(LifecycleSnapshot),
    Reattach(OwnedObjectPath),
}

fn apply_logind_display_change(
    current_session_path: &OwnedObjectPath,
    session_monitor_attached: bool,
    last_active: bool,
    last_type: &str,
    change: LogindDisplayPathChange,
) -> LogindDisplayChangeAction {
    match change {
        LogindDisplayPathChange::Unchanged => LogindDisplayChangeAction::Ignore,
        LogindDisplayPathChange::Empty => {
            if last_active || !last_type.is_empty() {
                LogindDisplayChangeAction::EmitNoSessionAndDetach(snapshot_no_session())
            } else if session_monitor_attached {
                LogindDisplayChangeAction::DetachSessionMonitor
            } else {
                LogindDisplayChangeAction::Ignore
            }
        }
        LogindDisplayPathChange::Path(path) => {
            if path == *current_session_path && session_monitor_attached {
                LogindDisplayChangeAction::Ignore
            } else {
                LogindDisplayChangeAction::Reattach(path)
            }
        }
    }
}

async fn wait_for_logind_display_session_path(
    user_proxy: &zbus::Proxy<'_>,
    signals: &mut zbus::fdo::PropertiesChangedStream,
) -> Result<OwnedObjectPath, DynError> {
    println!("[Logind] Waiting for display session to become ready");
    loop {
        let display =
            parse_logind_object_path(user_proxy.get_property("Display").await?, "User.Display")?;
        if !is_logind_empty_object_path(&display) {
            println!("[Logind] Using display session path: {}", display.as_str());
            return Ok(display);
        }

        let signal = expect_some_or_fail_fast(
            signals.next().await,
            "[Lifecycle] logind user properties-changed stream terminated".to_string(),
            fail_fast_lifecycle_monitor,
        );
        let args = expect_or_fail_fast(
            signal.args(),
            |error| format!("[Lifecycle] Failed to decode logind user signal: {}", error),
            fail_fast_lifecycle_monitor,
        );
        let display_change = expect_or_fail_fast(
            decode_logind_display_path_change(args.changed_properties.get("Display")),
            |error| error,
            fail_fast_lifecycle_monitor,
        );
        if let LogindDisplayPathChange::Path(display) = display_change {
            println!("[Logind] Using display session path: {}", display.as_str());
            return Ok(display);
        }
    }
}


#[derive(Debug)]
struct StartupSnapshotProvider {
    snapshot: Option<LifecycleSnapshot>,
}

impl StartupSnapshotProvider {
    fn new(env: Environment) -> Self {
        Self {
            snapshot: Some(startup_environment_to_snapshot(env)),
        }
    }

    async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        self.snapshot.take()
    }
}

#[derive(Debug)]
struct LogindLifecycleProvider {
    receiver: mpsc::UnboundedReceiver<LifecycleSnapshot>,
}

impl LogindLifecycleProvider {
    async fn new() -> Result<Self, DynError> {
        let connection = Connection::system().await?;
        verify_logind_lifecycle_monitor_prerequisites(&connection).await?;
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if let Err(error) = monitor_logind_lifecycle(connection, sender).await {
                fail_fast_lifecycle_monitor::<()>(format!(
                    "[Lifecycle] Failed to initialize logind lifecycle monitor: {}",
                    error
                ));
            }
        });

        Ok(Self { receiver })
    }

    async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        self.receiver.recv().await
    }
}

async fn verify_logind_lifecycle_monitor_prerequisites(
    connection: &Connection,
) -> Result<(), DynError> {
    let manager = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await?;
    let user_reply = manager
        .call_method("GetUserByPID", &(std::process::id()))
        .await?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")?;

    let _user_proxy = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await?;
    let user_properties_proxy = zbus::fdo::PropertiesProxy::builder(connection)
        .destination(LOGIND_BUS_NAME)?
        .path(user_path)?
        .build()
        .await?;
    let _user_signals = user_properties_proxy.receive_properties_changed().await?;

    match resolve_logind_session_path(connection).await {
        Ok(session_path) => {
            let _ = open_logind_session_monitor(connection, &session_path).await?;
        }
        Err(LogindSessionPathResolutionError::DisplayNotReady) => {}
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    }

    Ok(())
}

fn validate_active_logind_session_type(
    active: bool,
    session_type: &str,
) -> Result<(), &'static str> {
    if active && session_type.trim().is_empty() {
        return Err("[Lifecycle] logind Type property is empty for an active session");
    }
    Ok(())
}

fn fail_fast_lifecycle_monitor<T>(message: String) -> T {
    eprintln!("{}", message);
    std::process::exit(1);
}

fn expect_some_or_fail_fast<T, FF>(value: Option<T>, message: String, fail_fast: FF) -> T
where
    FF: FnOnce(String) -> T,
{
    match value {
        Some(value) => value,
        None => fail_fast(message),
    }
}

fn expect_or_fail_fast<T, E, MF, FF>(result: Result<T, E>, map_error: MF, fail_fast: FF) -> T
where
    MF: FnOnce(E) -> String,
    FF: FnOnce(String) -> T,
{
    match result {
        Ok(value) => value,
        Err(error) => fail_fast(map_error(error)),
    }
}

async fn monitor_logind_lifecycle(
    connection: Connection,
    sender: mpsc::UnboundedSender<LifecycleSnapshot>,
) -> Result<(), DynError> {
    let manager = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await?;
    let user_reply = manager
        .call_method("GetUserByPID", &(std::process::id()))
        .await?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")?;

    let user_proxy = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await?;
    let user_properties_proxy = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(LOGIND_BUS_NAME)?
        .path(user_path)?
        .build()
        .await?;
    let mut user_signals = user_properties_proxy.receive_properties_changed().await?;

    let mut session_path = match resolve_logind_session_path(&connection).await {
        Ok(path) => path,
        Err(LogindSessionPathResolutionError::DisplayNotReady) => {
            wait_for_logind_display_session_path(&user_proxy, &mut user_signals).await?
        }
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    };

    let (initial, session_signals) =
        open_logind_session_monitor(&connection, &session_path).await?;
    let mut session_signals = Some(session_signals);
    let mut last_active = initial.active;
    let mut last_type = initial.session_type.clone();
    sender
        .send(initial)
        .expect("lifecycle receiver dropped during provider init");

    loop {
        tokio::select! {
            user_signal = user_signals.next() => {
                let signal = expect_some_or_fail_fast(
                    user_signal,
                    "[Lifecycle] logind user properties-changed stream terminated".to_string(),
                    fail_fast_lifecycle_monitor,
                );
                let args = expect_or_fail_fast(
                    signal.args(),
                    |error| format!("[Lifecycle] Failed to decode logind user signal: {}", error),
                    fail_fast_lifecycle_monitor,
                );
                let change = expect_or_fail_fast(
                    decode_logind_display_path_change(args.changed_properties.get("Display")),
                    |error| error,
                    fail_fast_lifecycle_monitor,
                );
                match apply_logind_display_change(
                    &session_path,
                    session_signals.is_some(),
                    last_active,
                    &last_type,
                    change,
                ) {
                    LogindDisplayChangeAction::Ignore => {}
                    LogindDisplayChangeAction::DetachSessionMonitor => {
                        session_signals = None;
                    }
                    LogindDisplayChangeAction::EmitNoSessionAndDetach(snapshot) => {
                        session_signals = None;
                        last_active = snapshot.active;
                        last_type = snapshot.session_type.clone();
                        if sender.send(snapshot).is_err() {
                            return Ok(());
                        }
                    }
                    LogindDisplayChangeAction::Reattach(next_session_path) => {
                        println!(
                            "[Logind] Reattaching lifecycle monitor to display session path: {}",
                            next_session_path.as_str()
                        );
                        let (snapshot, next_signals) =
                            open_logind_session_monitor(&connection, &next_session_path).await?;
                        session_path = next_session_path;
                        session_signals = Some(next_signals);
                        last_active = snapshot.active;
                        last_type = snapshot.session_type.clone();
                        if sender.send(snapshot).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
            session_signal = async {
                match session_signals.as_mut() {
                    Some(signals) => signals.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let signal = expect_some_or_fail_fast(
                    session_signal,
                    "[Lifecycle] logind properties-changed stream terminated".to_string(),
                    fail_fast_lifecycle_monitor,
                );
                let args = expect_or_fail_fast(
                    signal.args(),
                    |error| format!("[Lifecycle] Failed to decode logind signal: {}", error),
                    fail_fast_lifecycle_monitor,
                );
                let snapshot = expect_or_fail_fast(
                    decode_logind_lifecycle_snapshot_change(
                        last_active,
                        &last_type,
                        args.changed_properties.get("Active"),
                        args.changed_properties.get("Type"),
                    ),
                    |error| error,
                    fail_fast_lifecycle_monitor,
                );
                let Some(snapshot) = snapshot else {
                    continue;
                };
                last_active = snapshot.active;
                last_type = snapshot.session_type.clone();
                if sender.send(snapshot).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

async fn open_logind_session_monitor(
    connection: &Connection,
    session_path: &OwnedObjectPath,
) -> Result<(LifecycleSnapshot, zbus::fdo::PropertiesChangedStream), DynError> {
    let session_proxy = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        session_path.clone(),
        LOGIND_SESSION_INTERFACE,
    )
    .await?;
    let active: bool = session_proxy.get_property("Active").await?;
    let session_type: String = session_proxy.get_property("Type").await?;
    validate_active_logind_session_type(active, &session_type).map_err(std::io::Error::other)?;

    let properties_proxy = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(LOGIND_BUS_NAME)?
        .path(session_path)?
        .build()
        .await?;
    let signals = properties_proxy.receive_properties_changed().await?;
    let initial = LifecycleSnapshot {
        active,
        session_type: session_type.clone(),
        session_kind: session_type_to_session_kind(active, &session_type),
    };
    Ok((initial, signals))
}

fn decode_logind_lifecycle_snapshot_change(
    last_active: bool,
    last_type: &str,
    active_value: Option<&Value<'_>>,
    type_value: Option<&Value<'_>>,
) -> Result<Option<LifecycleSnapshot>, String> {
    let mut next_active = last_active;
    let mut next_type = last_type.to_string();
    let mut changed = false;

    if let Some(value) = active_value {
        let parsed_active = value
            .downcast_ref::<bool>()
            .map_err(|_| "[Lifecycle] Failed to parse logind Active property".to_string())?;
        if parsed_active != last_active {
            next_active = parsed_active;
            changed = true;
        }
    }

    if let Some(value) = type_value {
        let parsed_type = if let Ok(parsed) = value.downcast_ref::<String>() {
            parsed
        } else if let Ok(parsed) = value.downcast_ref::<Str<'_>>() {
            parsed.to_string()
        } else {
            return Err("[Lifecycle] Failed to parse logind Type property".to_string());
        };
        if parsed_type != last_type {
            next_type = parsed_type;
            changed = true;
        }
    }

    if !changed {
        return Ok(None);
    }
    validate_active_logind_session_type(next_active, &next_type)
        .map_err(std::string::ToString::to_string)?;

    Ok(Some(LifecycleSnapshot {
        active: next_active,
        session_type: next_type.clone(),
        session_kind: session_type_to_session_kind(next_active, &next_type),
    }))
}

#[derive(Debug)]
enum LifecycleProvider {
    Logind(LogindLifecycleProvider),
    Startup(StartupSnapshotProvider),
}

impl LifecycleProvider {
    async fn build(env: Environment) -> Self {
        Self::build_with_logind_factory(env, LogindLifecycleProvider::new).await
    }

    async fn build_with_logind_factory<F, Fut>(env: Environment, logind_factory: F) -> Self
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<LogindLifecycleProvider, DynError>>,
    {
        match logind_factory().await {
            Ok(provider) => {
                println!("[Lifecycle] Provider=logind (continuous)");
                Self::Logind(provider)
            }
            Err(error) => {
                eprintln!(
                    "[Lifecycle] Provider=startup-snapshot (logind unavailable): {}",
                    error
                );
                Self::Startup(StartupSnapshotProvider::new(env))
            }
        }
    }

    fn is_continuous(&self) -> bool {
        matches!(self, Self::Logind(_))
    }

    async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        match self {
            Self::Logind(provider) => provider.next_snapshot().await,
            Self::Startup(provider) => provider.next_snapshot().await,
        }
    }
}

fn map_run_outcome_to_backend_exit(outcome: RunOutcome) -> BackendExit {
    match outcome {
        RunOutcome::Restart => BackendExit::Restart,
        RunOutcome::Exit => BackendExit::Exit,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendExit {
    Restart,
    Exit,
}

#[derive(Clone)]
struct BackendContext {
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    install_gnome_extension: bool,
    gnome_setup_completed: Arc<AtomicBool>,
    gnome_setup_hook: Arc<dyn Fn(bool) + Send + Sync>,
    /// Effective per-instance well-known DBus name owned by this daemon.
    /// Threaded into the GNOME signal-match filter and the KWin script template.
    effective_dbus_name: String,
}

struct BackendHandle {
    kind: BackendKind,
    shutdown_handle: ShutdownHandle,
    join_handle: Option<tokio::task::JoinHandle<Result<BackendExit, DynError>>>,
    finished_rx: watch::Receiver<bool>,
}

impl BackendHandle {
    fn is_finished(&self) -> bool {
        match &self.join_handle {
            Some(join_handle) => join_handle.is_finished(),
            None => true,
        }
    }

    async fn take_join_result(&mut self) -> Result<BackendExit, DynError> {
        let join_handle = self
            .join_handle
            .take()
            .expect("backend join handle missing");
        let result = join_handle.await.map_err(|error| {
            let message = format!("[Lifecycle] Backend task join failure: {}", error);
            Box::<dyn std::error::Error + Send + Sync>::from(message)
        })?;
        result
    }

    async fn stop(&mut self) -> Result<BackendExit, DynError> {
        if self.join_handle.is_none() {
            return Ok(BackendExit::Exit);
        }
        self.shutdown_handle.request();
        self.take_join_result().await
    }

    fn finished_receiver(&self) -> watch::Receiver<bool> {
        self.finished_rx.clone()
    }
}

fn runtime_target_label(target: RuntimeTarget) -> &'static str {
    match target {
        RuntimeTarget::Idle => "idle",
        RuntimeTarget::Backend(BackendKind::Gnome) => "gnome",
        RuntimeTarget::Backend(BackendKind::Kde) => "kde",
        RuntimeTarget::Backend(BackendKind::Wayland) => "wayland",
        RuntimeTarget::Backend(BackendKind::X11) => "x11",
        RuntimeTarget::Backend(BackendKind::LinuxConsole) => "linux-console",
    }
}

fn runtime_target_is_wayland_family(target: RuntimeTarget) -> bool {
    matches!(
        target,
        RuntimeTarget::Backend(BackendKind::Gnome)
            | RuntimeTarget::Backend(BackendKind::Kde)
            | RuntimeTarget::Backend(BackendKind::Wayland)
    )
}

fn runtime_target_to_environment(target: RuntimeTarget) -> Environment {
    match target {
        RuntimeTarget::Backend(BackendKind::Gnome) => Environment::Gnome,
        RuntimeTarget::Backend(BackendKind::Kde) => Environment::Kde,
        RuntimeTarget::Backend(BackendKind::Wayland) => Environment::Wayland,
        RuntimeTarget::Backend(BackendKind::X11) => Environment::X11,
        RuntimeTarget::Backend(BackendKind::LinuxConsole) => Environment::LinuxConsoleWithLogind,
        RuntimeTarget::Idle => Environment::Unknown,
    }
}

const WAYLAND_CAPABILITY_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

async fn detect_desktop_capabilities() -> Result<DesktopCapabilities, DynError> {
    let connection = Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    let gnome_owner = session_bus_name_has_owner(&dbus, GNOME_SHELL_BUS_NAME).await;
    let kde_owner = session_bus_name_has_owner(&dbus, KDE_KWIN_BUS_NAME).await;
    Ok(DesktopCapabilities {
        gnome_owner,
        kde_owner,
    })
}

async fn resolve_runtime_target_for_snapshot(
    snapshot: &LifecycleSnapshot,
) -> Result<RuntimeTarget, DynError> {
    let capabilities = if snapshot.session_kind == SessionKind::GraphicalWayland {
        if let Some(hinted_target) =
            runtime_target_from_wayland_startup_session_type_hint(snapshot.session_type.as_str())
        {
            return Ok(hinted_target);
        }
        detect_desktop_capabilities().await?
    } else {
        DesktopCapabilities {
            gnome_owner: false,
            kde_owner: false,
        }
    };
    Ok(resolve_runtime_target(snapshot.session_kind, capabilities))
}

async fn run_gnome_backend_task(
    context: BackendContext,
    shutdown_handle: ShutdownHandle,
) -> Result<BackendExit, DynError> {
    let outcome = run_gnome(
        context.kanata,
        context.handler,
        context.status_broadcaster,
        context.restart_handle,
        context.pause_broadcaster,
        shutdown_handle,
    )
    .await?;
    Ok(map_run_outcome_to_backend_exit(outcome))
}

async fn run_kde_backend_task(
    context: BackendContext,
    shutdown_handle: ShutdownHandle,
) -> Result<BackendExit, DynError> {
    let outcome = run_kde(
        context.kanata,
        context.handler,
        context.status_broadcaster,
        context.restart_handle,
        context.pause_broadcaster,
        shutdown_handle,
        context.effective_dbus_name,
    )
    .await?;
    Ok(map_run_outcome_to_backend_exit(outcome))
}

async fn run_wayland_backend_task(
    context: BackendContext,
    wayland_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<BackendExit, DynError> {
    run_wayland(
        context.kanata,
        context.handler,
        context.status_broadcaster,
        context.pause_broadcaster,
        wayland_display_override,
        shutdown_handle,
    )
    .await?;
    Ok(BackendExit::Exit)
}

async fn run_x11_backend_task(
    context: BackendContext,
    x11_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<BackendExit, DynError> {
    run_x11(
        context.kanata,
        context.handler,
        context.status_broadcaster,
        context.pause_broadcaster,
        x11_display_override,
        shutdown_handle,
    )
    .await?;
    Ok(BackendExit::Exit)
}

async fn run_linux_console_backend_task(
    context: BackendContext,
    shutdown_handle: ShutdownHandle,
) -> Result<BackendExit, DynError> {
    apply_focus_for_env(
        Environment::LinuxConsoleWithLogind,
        None,
        false,
        &context.handler,
        &context.status_broadcaster,
        &context.pause_broadcaster,
        &context.kanata,
    )
    .await?;
    let outcome = wait_for_restart_or_shutdown(&context.restart_handle, &shutdown_handle).await;
    Ok(map_run_outcome_to_backend_exit(outcome))
}

async fn ensure_runtime_gnome_extension_setup(context: &BackendContext) -> Result<(), DynError> {
    if context.gnome_setup_completed.load(Ordering::SeqCst) {
        return Ok(());
    }

    let setup_hook = context.gnome_setup_hook.clone();
    let install_gnome_extension = context.install_gnome_extension;
    tokio::task::spawn_blocking(move || (setup_hook)(install_gnome_extension))
        .await
        .map_err(|error| -> DynError {
            format!("[GNOME] Extension setup task failed: {}", error).into()
        })?;

    context.gnome_setup_completed.store(true, Ordering::SeqCst);
    Ok(())
}

fn display_override_expected_session_type(kind: BackendKind) -> Option<&'static str> {
    match kind {
        BackendKind::Wayland => Some("wayland"),
        BackendKind::X11 => Some("x11"),
        BackendKind::Gnome | BackendKind::Kde | BackendKind::LinuxConsole => None,
    }
}

fn is_valid_wayland_display_override(display: &str) -> bool {
    if Path::new(display).is_absolute() {
        return true;
    }
    if display.starts_with(':') {
        return false;
    }
    if display.contains('/') {
        return false;
    }
    true
}

fn normalize_display_override(kind: BackendKind, display: &str) -> Option<String> {
    let trimmed = display.trim();
    if trimmed.is_empty() {
        return None;
    }
    if kind == BackendKind::Wayland && !is_valid_wayland_display_override(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

async fn resolve_display_override_from_logind(
    kind: BackendKind,
) -> Result<Option<String>, DynError> {
    let expected_type = match display_override_expected_session_type(kind) {
        Some(value) => value,
        None => return Ok(None),
    };

    let system_connection = Connection::system().await?;
    let session_path = match resolve_logind_session_path(&system_connection).await {
        Ok(path) => path,
        Err(LogindSessionPathResolutionError::DisplayNotReady) => return Ok(None),
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    };
    let session_proxy = zbus::Proxy::new(
        &system_connection,
        LOGIND_BUS_NAME,
        session_path.as_str(),
        LOGIND_SESSION_INTERFACE,
    )
    .await?;
    let session_type: String = session_proxy.get_property("Type").await?;
    if session_type != expected_type {
        return Ok(None);
    }

    let display: String = session_proxy.get_property("Display").await?;
    let normalized = normalize_display_override(kind, &display);
    if kind == BackendKind::Wayland && normalized.is_none() && !display.trim().is_empty() {
        eprintln!(
            "[Lifecycle] Ignoring logind Wayland Display override '{}': not a valid Wayland socket value",
            display.trim()
        );
    }
    Ok(normalized)
}

fn display_override_backend_kind_for_environment(env: Environment) -> Option<BackendKind> {
    match env {
        Environment::Wayland => Some(BackendKind::Wayland),
        Environment::X11 => Some(BackendKind::X11),
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => None,
    }
}

#[cfg(test)]
static TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
static TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
struct TestFocusQueryDisplayOverrideGuard {
    env: Environment,
    previous: Option<String>,
}

#[cfg(test)]
impl Drop for TestFocusQueryDisplayOverrideGuard {
    fn drop(&mut self) {
        if let Some(slot) = display_override_test_slot(self.env) {
            *slot.lock().unwrap() = self.previous.clone();
        }
    }
}

#[cfg(test)]
fn display_override_test_slot(
    env: Environment,
) -> Option<&'static std::sync::Mutex<Option<String>>> {
    match env {
        Environment::X11 => Some(&TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE),
        Environment::Wayland => Some(&TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE),
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => None,
    }
}

#[cfg(test)]
fn set_test_focus_query_display_override(
    env: Environment,
    override_value: Option<&str>,
) -> TestFocusQueryDisplayOverrideGuard {
    let slot = display_override_test_slot(env)
        .expect("focus-query test display override is only valid for X11/Wayland");
    let mut guard = slot.lock().unwrap();
    let previous = guard.clone();
    *guard = override_value.map(str::to_string);
    TestFocusQueryDisplayOverrideGuard { env, previous }
}

#[cfg(test)]
fn resolve_test_focus_query_display_override(env: Environment) -> Option<String> {
    let slot = match display_override_test_slot(env) {
        Some(slot) => slot,
        None => return None,
    };
    slot.lock().unwrap().clone()
}

#[cfg(not(test))]
fn resolve_test_focus_query_display_override(_env: Environment) -> Option<String> {
    None
}

async fn resolve_display_override_for_backend_kind(
    kind: BackendKind,
    context_label: &str,
) -> Option<String> {
    let expected_type = match display_override_expected_session_type(kind) {
        Some(value) => value,
        None => return None,
    };
    match resolve_display_override_from_logind(kind).await {
        Ok(Some(display)) => {
            println!(
                "[{}] Refreshed {} display endpoint from logind: {}",
                context_label, expected_type, display
            );
            Some(display)
        }
        Ok(None) => None,
        Err(error) => {
            eprintln!(
                "[{}] Failed to refresh {} display endpoint from logind: {}",
                context_label, expected_type, error
            );
            None
        }
    }
}

async fn resolve_display_override_for_environment(
    env: Environment,
    context_label: &str,
) -> Option<String> {
    if let Some(display) = resolve_test_focus_query_display_override(env) {
        return Some(display);
    }
    let kind = match display_override_backend_kind_for_environment(env) {
        Some(kind) => kind,
        None => return None,
    };
    resolve_display_override_for_backend_kind(kind, context_label).await
}

async fn start_backend(
    kind: BackendKind,
    context: &BackendContext,
) -> Result<BackendHandle, DynError> {
    let shutdown_handle = ShutdownHandle::new();
    let (finished_tx, finished_rx) = watch::channel(false);
    let join_handle = match kind {
        BackendKind::Gnome => {
            let task_context = context.clone();
            let task_shutdown = shutdown_handle.clone();
            let task_finished = finished_tx.clone();
            tokio::spawn(async move {
                let result = run_gnome_backend_task(task_context, task_shutdown).await;
                let _ = task_finished.send(true);
                result
            })
        }
        BackendKind::Kde => {
            let task_context = context.clone();
            let task_shutdown = shutdown_handle.clone();
            let task_finished = finished_tx.clone();
            tokio::spawn(async move {
                let result = run_kde_backend_task(task_context, task_shutdown).await;
                let _ = task_finished.send(true);
                result
            })
        }
        BackendKind::Wayland => {
            let wayland_display_override =
                resolve_display_override_for_backend_kind(kind, "Lifecycle").await;
            let task_context = context.clone();
            let task_shutdown = shutdown_handle.clone();
            let task_finished = finished_tx.clone();
            tokio::spawn(async move {
                let result =
                    run_wayland_backend_task(task_context, wayland_display_override, task_shutdown)
                        .await;
                let _ = task_finished.send(true);
                result
            })
        }
        BackendKind::X11 => {
            let x11_display_override =
                resolve_display_override_for_backend_kind(kind, "Lifecycle").await;
            let task_context = context.clone();
            let task_shutdown = shutdown_handle.clone();
            let task_finished = finished_tx.clone();
            tokio::spawn(async move {
                let result =
                    run_x11_backend_task(task_context, x11_display_override, task_shutdown).await;
                let _ = task_finished.send(true);
                result
            })
        }
        BackendKind::LinuxConsole => {
            let task_context = context.clone();
            let task_shutdown = shutdown_handle.clone();
            let task_finished = finished_tx.clone();
            tokio::spawn(async move {
                let result = run_linux_console_backend_task(task_context, task_shutdown).await;
                let _ = task_finished.send(true);
                result
            })
        }
    };

    Ok(BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    })
}

struct SupervisorState {
    current_target: RuntimeTarget,
    backend: Option<BackendHandle>,
}

impl SupervisorState {
    fn new() -> Self {
        Self {
            current_target: RuntimeTarget::Idle,
            backend: None,
        }
    }
}

#[cfg(test)]
async fn transition_runtime_target(
    state: &mut SupervisorState,
    desired_target: RuntimeTarget,
    context: &BackendContext,
    reason: &str,
) -> Result<(), DynError> {
    transition_runtime_target_with_starter(
        state,
        desired_target,
        context,
        reason,
        |kind, context| async move { start_backend(kind, &context).await },
    )
    .await
}

async fn transition_runtime_target_with_starter<F, Fut>(
    state: &mut SupervisorState,
    desired_target: RuntimeTarget,
    context: &BackendContext,
    reason: &str,
    starter: F,
) -> Result<(), DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
{
    if state.current_target == desired_target {
        return Ok(());
    }
    let requires_session_bus = target_requires_session_bus(desired_target);

    println!(
        "[LifecycleTransition] from={} to={} session_bus_required={} reason={}",
        runtime_target_label(state.current_target),
        runtime_target_label(desired_target),
        requires_session_bus,
        reason
    );

    if let Some(mut backend) = state.backend.take() {
        let exit = backend.stop().await?;
        if exit == BackendExit::Restart {
            context.restart_handle.request();
        }
    }

    if desired_target == RuntimeTarget::Backend(BackendKind::Gnome) {
        ensure_runtime_gnome_extension_setup(context).await?;
    }

    if let RuntimeTarget::Backend(kind) = desired_target {
        let backend = starter(kind, context.clone()).await?;
        state.backend = Some(backend);
    }

    state.current_target = desired_target;
    context
        .runtime_environment
        .set_current(runtime_target_to_environment(desired_target));
    Ok(())
}

async fn stop_current_backend(
    state: &mut SupervisorState,
    context: &BackendContext,
) -> Result<(), DynError> {
    if let Some(mut backend) = state.backend.take() {
        let exit = backend.stop().await?;
        if exit == BackendExit::Restart {
            context.restart_handle.request();
        }
    }
    state.current_target = RuntimeTarget::Idle;
    context
        .runtime_environment
        .set_current(Environment::Unknown);
    Ok(())
}

async fn run_lifecycle_supervisor(
    provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
) -> Result<RunOutcome, DynError> {
    run_lifecycle_supervisor_with_starter(
        provider,
        context,
        restart_handle,
        shutdown_handle,
        |kind, context| async move { start_backend(kind, &context).await },
    )
    .await
}

async fn run_lifecycle_supervisor_with_starter<F, Fut>(
    provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    starter: F,
) -> Result<RunOutcome, DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
{
    run_lifecycle_supervisor_with_starter_and_resolver(
        provider,
        context,
        restart_handle,
        shutdown_handle,
        starter,
        |snapshot| async move { resolve_runtime_target_for_snapshot(&snapshot).await },
        WAYLAND_CAPABILITY_RECHECK_INTERVAL,
    )
    .await
}

async fn run_lifecycle_supervisor_with_starter_and_resolver<F, Fut, R, RFut>(
    mut provider: LifecycleProvider,
    context: BackendContext,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    starter: F,
    resolver: R,
    wayland_capability_recheck_interval: Duration,
) -> Result<RunOutcome, DynError>
where
    F: Fn(BackendKind, BackendContext) -> Fut,
    Fut: std::future::Future<Output = Result<BackendHandle, DynError>>,
    R: Fn(LifecycleSnapshot) -> RFut,
    RFut: std::future::Future<Output = Result<RuntimeTarget, DynError>>,
{
    let mut state = SupervisorState::new();
    let allow_wayland_capability_recheck = provider.is_continuous();
    context
        .runtime_environment
        .set_current(runtime_target_to_environment(state.current_target));
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();
    let mut provider_open = true;
    let mut last_snapshot: Option<LifecycleSnapshot> = None;

    loop {
        if *shutdown_receiver.borrow() {
            stop_current_backend(&mut state, &context).await?;
            return Ok(RunOutcome::Exit);
        }
        if *restart_receiver.borrow() {
            stop_current_backend(&mut state, &context).await?;
            return Ok(RunOutcome::Restart);
        }

        if let Some(outcome) = poll_finished_backend_outcome(&mut state).await? {
            return Ok(outcome);
        }

        let mut backend_finished = state
            .backend
            .as_ref()
            .map(|backend| backend.finished_receiver());
        if provider_open {
            tokio::select! {
                _ = shutdown_receiver.changed() => {}
                _ = restart_receiver.changed() => {}
                _ = wait_for_backend_completion_signal(&mut backend_finished) => {}
                _ = wait_for_wayland_capability_recheck(
                    allow_wayland_capability_recheck,
                    last_snapshot.as_ref(),
                    wayland_capability_recheck_interval,
                ) => {
                    let snapshot = last_snapshot
                        .clone()
                        .expect("capability recheck requires last snapshot");
                    match resolver(snapshot).await {
                        Ok(desired_target) => {
                            transition_runtime_target_with_starter(
                                &mut state,
                                desired_target,
                                &context,
                                "wayland-capability-recheck",
                                &starter,
                            )
                            .await?;
                        }
                        Err(error) => {
                            if runtime_target_is_wayland_family(state.current_target) {
                                eprintln!(
                                    "[Lifecycle] Keeping {} backend after capability recheck resolver error: {}",
                                    runtime_target_label(state.current_target),
                                    error
                                );
                            } else {
                                eprintln!(
                                    "[Lifecycle] Falling back to generic Wayland after capability recheck resolver error: {}",
                                    error
                                );
                                transition_runtime_target_with_starter(
                                    &mut state,
                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                    &context,
                                    "wayland-capability-recheck-fallback-after-resolver-error",
                                    &starter,
                                )
                                .await?;
                            }
                        }
                    }
                }
                next_snapshot = provider.next_snapshot() => {
                    match next_snapshot {
                        Some(snapshot) => {
                            last_snapshot = Some(snapshot.clone());
                            match resolver(snapshot.clone()).await {
                                Ok(desired_target) => {
                                    let reason = format!(
                                        "active={} type={} kind={:?}",
                                        snapshot.active,
                                        snapshot.session_type,
                                        snapshot.session_kind
                                    );
                                    transition_runtime_target_with_starter(
                                        &mut state,
                                        desired_target,
                                        &context,
                                        &reason,
                                        &starter,
                                    )
                                    .await?;
                                }
                                Err(error) => {
                                    if snapshot.session_kind == SessionKind::GraphicalWayland {
                                        if allow_wayland_capability_recheck {
                                            if runtime_target_is_wayland_family(state.current_target) {
                                                eprintln!(
                                                    "[Lifecycle] Keeping {} backend after continuous wayland resolver error: {}",
                                                    runtime_target_label(state.current_target),
                                                    error
                                                );
                                            } else {
                                                eprintln!(
                                                    "[Lifecycle] Falling back to generic Wayland after continuous resolver error: {}",
                                                    error
                                                );
                                                transition_runtime_target_with_starter(
                                                    &mut state,
                                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                                    &context,
                                                    "continuous-wayland-fallback-after-resolver-error",
                                                    &starter,
                                                )
                                                .await?;
                                            }
                                        } else {
                                            eprintln!(
                                                "[Lifecycle] Falling back to generic Wayland after startup resolver error: {}",
                                                error
                                            );
                                            transition_runtime_target_with_starter(
                                                &mut state,
                                                RuntimeTarget::Backend(BackendKind::Wayland),
                                                &context,
                                                "startup-wayland-fallback-after-resolver-error",
                                                &starter,
                                            )
                                            .await?;
                                        }
                                    } else if allow_wayland_capability_recheck {
                                        eprintln!(
                                            "[Lifecycle] Skipping transition after resolver error: {}",
                                            error
                                        );
                                    } else {
                                        return Err(format!(
                                            "[Lifecycle] Startup lifecycle target resolution failed: {}",
                                            error
                                        )
                                        .into());
                                    }
                                }
                            }
                        }
                        None => {
                            provider_open = false;
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                _ = shutdown_receiver.changed() => {}
                _ = restart_receiver.changed() => {}
                _ = wait_for_backend_completion_signal(&mut backend_finished) => {}
                _ = wait_for_wayland_capability_recheck(
                    allow_wayland_capability_recheck,
                    last_snapshot.as_ref(),
                    wayland_capability_recheck_interval,
                ) => {
                    let snapshot = last_snapshot
                        .clone()
                        .expect("capability recheck requires last snapshot");
                    match resolver(snapshot).await {
                        Ok(desired_target) => {
                            transition_runtime_target_with_starter(
                                &mut state,
                                desired_target,
                                &context,
                                "wayland-capability-recheck",
                                &starter,
                            )
                            .await?;
                        }
                        Err(error) => {
                            if runtime_target_is_wayland_family(state.current_target) {
                                eprintln!(
                                    "[Lifecycle] Keeping {} backend after capability recheck resolver error: {}",
                                    runtime_target_label(state.current_target),
                                    error
                                );
                            } else {
                                eprintln!(
                                    "[Lifecycle] Falling back to generic Wayland after capability recheck resolver error: {}",
                                    error
                                );
                                transition_runtime_target_with_starter(
                                    &mut state,
                                    RuntimeTarget::Backend(BackendKind::Wayland),
                                    &context,
                                    "wayland-capability-recheck-fallback-after-resolver-error",
                                    &starter,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn wait_for_wayland_capability_recheck(
    enabled: bool,
    last_snapshot: Option<&LifecycleSnapshot>,
    interval: Duration,
) {
    if !enabled {
        std::future::pending::<()>().await;
        return;
    }
    let Some(snapshot) = last_snapshot else {
        std::future::pending::<()>().await;
        return;
    };
    if snapshot.session_kind != SessionKind::GraphicalWayland {
        std::future::pending::<()>().await;
        return;
    }
    tokio::time::sleep(interval).await;
}

async fn wait_for_backend_completion_signal(backend_finished: &mut Option<watch::Receiver<bool>>) {
    let Some(receiver) = backend_finished.as_mut() else {
        std::future::pending::<()>().await;
        return;
    };
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await;
}

async fn poll_finished_backend_outcome(
    state: &mut SupervisorState,
) -> Result<Option<RunOutcome>, DynError> {
    let Some(backend) = state.backend.as_mut() else {
        return Ok(None);
    };
    if !backend.is_finished() {
        return Ok(None);
    }

    let exit = backend.take_join_result().await?;
    let kind = backend.kind;
    state.backend = None;
    state.current_target = RuntimeTarget::Idle;
    match exit {
        BackendExit::Restart => Ok(Some(RunOutcome::Restart)),
        BackendExit::Exit => {
            Err(format!("[Lifecycle] backend {:?} exited unexpectedly", kind).into())
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
    record_unpause_request_environment_for_test(env);
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

#[cfg(test)]
static TEST_LAST_UNPAUSE_REQUEST_ENV: std::sync::Mutex<Option<Environment>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn record_unpause_request_environment_for_test(env: Environment) {
    *TEST_LAST_UNPAUSE_REQUEST_ENV.lock().unwrap() = Some(env);
}

#[cfg(not(test))]
fn record_unpause_request_environment_for_test(_env: Environment) {}

#[cfg(test)]
fn take_unpause_request_environment_for_test() -> Option<Environment> {
    TEST_LAST_UNPAUSE_REQUEST_ENV.lock().unwrap().take()
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
    wayland_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = connect_wayland_with_display_override(wayland_display_override.as_deref())?;
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
    fn new(
        display_override: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (connection, screen_num) = x11rb::connect(display_override)?;
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
    x11_display_override: Option<String>,
    shutdown_handle: ShutdownHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = X11State::new(x11_display_override.as_deref())?;

    println!("[X11] Connected to display");

    let initial = state.get_active_window();
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &initial,
        &kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(&kanata, actions).await;
    }

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
) -> Option<SniIndicatorRuntimeHandle> {
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
    let status_watch_task = tokio::spawn(async move {
        #[cfg(test)]
        let _watcher_guard = SniWatcherTaskGuard::new();
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
    let pause_watch_task = tokio::spawn(async move {
        #[cfg(test)]
        let _watcher_guard = SniWatcherTaskGuard::new();
        loop {
            if pause_receiver.changed().await.is_err() {
                break;
            }
            let paused = *pause_receiver.borrow();
            pause_handle.update(|state| state.set_paused(paused));
        }
    });

    let menu_handle = handle.clone();
    let menu_watch_task = tokio::spawn(async move {
        #[cfg(test)]
        let _watcher_guard = SniWatcherTaskGuard::new();
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

    Some(SniIndicatorRuntimeHandle {
        handle,
        status_watch_task,
        pause_watch_task,
        menu_watch_task,
    })
}

async fn build_sni_control_for_mode(
    mode: SniControlMode,
    runtime_handle: tokio::runtime::Handle,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    restart_handle: RestartHandle,
    shutdown_handle: ShutdownHandle,
    control_environment: Environment,
    daemon_bus_name: String,
) -> Option<SniControl> {
    match mode {
        SniControlMode::Local => Some(SniControl::Local(SniLocalControl {
            runtime_handle,
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            shutdown_handle,
            unpause_context: local_sni_unpause_context(control_environment),
        })),
        SniControlMode::Dbus => match Connection::session().await {
            Ok(connection) => Some(SniControl::Dbus(SniDbusControl {
                runtime_handle,
                connection,
                restart_handle,
                shutdown_handle,
                daemon_bus_name,
            })),
            Err(error) => {
                eprintln!("[SNI] Failed to connect to session bus: {}", error);
                None
            }
        },
    }
}

struct SniIndicatorRuntimeHandle {
    handle: ksni::Handle<SniIndicator>,
    status_watch_task: tokio::task::JoinHandle<()>,
    pause_watch_task: tokio::task::JoinHandle<()>,
    menu_watch_task: tokio::task::JoinHandle<()>,
}

impl SniIndicatorRuntimeHandle {
    fn shutdown(&self) {
        self.status_watch_task.abort();
        self.pause_watch_task.abort();
        self.menu_watch_task.abort();
        self.handle.shutdown();
    }
}

impl Drop for SniIndicatorRuntimeHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
static ACTIVE_SNI_WATCHER_TASKS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
struct SniWatcherTaskGuard;

#[cfg(test)]
impl SniWatcherTaskGuard {
    fn new() -> Self {
        ACTIVE_SNI_WATCHER_TASKS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for SniWatcherTaskGuard {
    fn drop(&mut self) {
        ACTIVE_SNI_WATCHER_TASKS.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
fn sni_watcher_task_count() -> usize {
    ACTIVE_SNI_WATCHER_TASKS.load(Ordering::SeqCst)
}

struct SniGuard {
    handle: Arc<Mutex<Option<SniIndicatorRuntimeHandle>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SniGuard {
    fn disabled() -> Self {
        Self {
            handle: Arc::new(Mutex::new(None)),
            task: None,
        }
    }

    fn runtime_managed(
        runtime_environment: RuntimeEnvironmentBroadcaster,
        runtime_handle: tokio::runtime::Handle,
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        status_broadcaster: StatusBroadcaster,
        pause_broadcaster: PauseBroadcaster,
        restart_handle: RestartHandle,
        shutdown_handle: ShutdownHandle,
        indicator_focus_only: Option<TrayFocusOnly>,
        daemon_bus_name: String,
    ) -> Self {
        Self::runtime_managed_with_builder(
            runtime_environment,
            runtime_handle,
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            shutdown_handle,
            indicator_focus_only,
            SNI_RUNTIME_RETRY_INTERVAL,
            build_sni_control_for_mode,
            daemon_bus_name,
        )
    }

    fn runtime_managed_with_builder<B, BFut>(
        runtime_environment: RuntimeEnvironmentBroadcaster,
        runtime_handle: tokio::runtime::Handle,
        kanata: KanataClient,
        handler: Arc<Mutex<FocusHandler>>,
        status_broadcaster: StatusBroadcaster,
        pause_broadcaster: PauseBroadcaster,
        restart_handle: RestartHandle,
        shutdown_handle: ShutdownHandle,
        indicator_focus_only: Option<TrayFocusOnly>,
        retry_delay: Duration,
        control_builder: B,
        daemon_bus_name: String,
    ) -> Self
    where
        B: Fn(
                SniControlMode,
                tokio::runtime::Handle,
                KanataClient,
                Arc<Mutex<FocusHandler>>,
                StatusBroadcaster,
                PauseBroadcaster,
                RestartHandle,
                ShutdownHandle,
                Environment,
                String,
            ) -> BFut
            + Send
            + Sync
            + 'static,
        BFut: std::future::Future<Output = Option<SniControl>> + Send + 'static,
    {
        let shared_handle: Arc<Mutex<Option<SniIndicatorRuntimeHandle>>> =
            Arc::new(Mutex::new(None));
        let task_handle_store = shared_handle.clone();
        let mut env_receiver = runtime_environment.subscribe();
        let task = tokio::spawn(async move {
            let mut active_mode: Option<SniControlMode> = None;
            let mut active_env: Option<Environment> = None;
            loop {
                let env = *env_receiver.borrow();
                match plan_sni_runtime_transition(active_mode, active_env, env) {
                    SniRuntimeTransitionPlan::Keep => {}
                    SniRuntimeTransitionPlan::Stop => {
                        if let Some(handle) = task_handle_store.lock().unwrap().take() {
                            println!("[SNI] Shutting down indicator");
                            drop(handle);
                        }
                        active_mode = None;
                        active_env = None;
                    }
                    SniRuntimeTransitionPlan::Start(mode)
                    | SniRuntimeTransitionPlan::Restart(mode) => {
                        if let Some(handle) = task_handle_store.lock().unwrap().take() {
                            println!("[SNI] Shutting down indicator");
                            drop(handle);
                        }
                        active_mode = None;
                        active_env = None;
                        let control = control_builder(
                            mode,
                            runtime_handle.clone(),
                            kanata.clone(),
                            handler.clone(),
                            status_broadcaster.clone(),
                            pause_broadcaster.clone(),
                            restart_handle.clone(),
                            shutdown_handle.clone(),
                            env,
                            daemon_bus_name.clone(),
                        )
                        .await;
                        if let Some(control) = control {
                            let handle = start_sni_indicator(
                                control,
                                status_broadcaster.clone(),
                                pause_broadcaster.clone(),
                                indicator_focus_only,
                            );
                            *task_handle_store.lock().unwrap() = handle;
                            active_mode = Some(mode);
                            active_env = Some(env);
                        } else {
                            eprintln!(
                                "[SNI] Failed to initialize {:?} control; retrying in {}ms unless environment changes",
                                mode,
                                retry_delay.as_millis()
                            );
                        }
                    }
                }

                let should_retry_start =
                    active_mode.is_none() && sni_control_mode_for_environment(env).is_some();
                if matches!(
                    wait_for_sni_runtime_wake_with_delay(
                        &mut env_receiver,
                        should_retry_start,
                        retry_delay,
                    )
                    .await,
                    SniRuntimeWakeReason::ChannelClosed
                ) {
                    break;
                }
            }
        });
        Self {
            handle: shared_handle,
            task: Some(task),
        }
    }
}

impl Drop for SniGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(handle) = self.handle.lock().unwrap().take() {
            println!("[SNI] Shutting down indicator");
            drop(handle);
        }
    }
}

fn dconf_get_bool(key: &str) -> Result<bool, String> {
    let output = Command::new("dconf")
        .args(["read", key])
        .output()
        .map_err(|error| format!("dconf read failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dconf read failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        "" => Err("key not set".to_string()),
        value => Err(format!("unexpected dconf output: {}", value)),
    }
}

fn dconf_set_bool(key: &str, value: bool) -> Result<(), String> {
    let value_str = if value { "true" } else { "false" };
    let output = Command::new("dconf")
        .args(["write", key, value_str])
        .output()
        .map_err(|error| format!("dconf write failed: {}", error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dconf write failed: {}", stderr.trim()));
    }

    Ok(())
}

fn is_dconf_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("no such file or directory")
        || lower.contains("not found")
        || lower.contains("failed to execute")
}

// === GNOME Extension Management ===

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
const EMBEDDED_DAEMON_STATE_JS: &str = include_str!(gnome_ext_file!("daemon-state.js"));
#[cfg(feature = "embed-gnome-extension")]
const EMBEDDED_MULTIPLEX_JS: &str = include_str!(gnome_ext_file!("extension-multiplex.js"));
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
        && path.join("daemon-state.js").exists()
        && path.join("extension-multiplex.js").exists()
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
    fs::write(dir.join("daemon-state.js"), EMBEDDED_DAEMON_STATE_JS)?;
    fs::write(dir.join("extension-multiplex.js"), EMBEDDED_MULTIPLEX_JS)?;
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
    /// Whether org.gnome.Shell is reachable on the session bus.
    shell_service_available: bool,
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
        shell_service_available: true,
        state: Some(state),
        method: GnomeDetectionMethod::Dbus,
    }
}

enum GnomeDbusProbeResult {
    Status(GnomeExtensionStatus),
    ShellUnavailable,
    ProbeFailed,
}

fn is_dbus_service_unavailable(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, description, _) => {
            name.as_ref() == DBUS_ERROR_SERVICE_UNKNOWN
                || name.as_ref() == DBUS_ERROR_NAME_HAS_NO_OWNER
                || (name.as_ref() == DBUS_ERROR_UNKNOWN_METHOD
                    && description
                        .as_deref()
                        .map(|message| {
                            message.contains("Object does not exist at path")
                                || message.contains("No such interface")
                        })
                        .unwrap_or(false))
        }
        _ => false,
    }
}

/// Quick probe: check if extension is active via D-Bus call to GNOME Shell.
/// This bypasses filesystem searches and works reliably from systemd services.
fn gnome_extension_dbus_probe() -> GnomeDbusProbeResult {
    let connection = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[GNOME] D-Bus probe: failed to connect to session bus: {}",
                e
            );
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };
    gnome_extension_dbus_probe_with_connection(&connection)
}

/// Probe using a specific D-Bus connection (for testing with mock services)
fn gnome_extension_dbus_probe_with_connection(
    connection: &zbus::blocking::Connection,
) -> GnomeDbusProbeResult {
    let reply = match connection.call_method(
        Some(GNOME_SHELL_BUS_NAME),
        GNOME_SHELL_OBJECT_PATH,
        Some(GNOME_SHELL_EXTENSIONS_INTERFACE),
        "GetExtensionInfo",
        &(GNOME_EXTENSION_UUID,),
    ) {
        Ok(r) => r,
        Err(e) => {
            if is_dbus_service_unavailable(&e) {
                println!("[GNOME] D-Bus probe: GNOME Shell D-Bus interface not ready yet");
                return GnomeDbusProbeResult::ShellUnavailable;
            }
            eprintln!("[GNOME] D-Bus probe: GetExtensionInfo call failed: {}", e);
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };

    // Response is a dict (a{sv}) with extension info
    let body: HashMap<String, zbus::zvariant::OwnedValue> = match reply.body().deserialize() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[GNOME] D-Bus probe: failed to deserialize response: {}", e);
            return GnomeDbusProbeResult::ProbeFailed;
        }
    };

    GnomeDbusProbeResult::Status(parse_gnome_extension_state(&body))
}

fn gnome_extension_status() -> GnomeExtensionStatus {
    // Quick probe: try D-Bus call to GNOME Shell first
    // This is the most reliable method from systemd services
    match gnome_extension_dbus_probe() {
        GnomeDbusProbeResult::Status(status) => return status,
        GnomeDbusProbeResult::ShellUnavailable => {
            return GnomeExtensionStatus {
                installed: false,
                enabled: false,
                active: false,
                shell_service_available: false,
                state: None,
                method: GnomeDetectionMethod::Dbus,
            };
        }
        GnomeDbusProbeResult::ProbeFailed => {}
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
        shell_service_available: true,
        state: None,
        method: GnomeDetectionMethod::Cli,
    }
}

fn wait_for_session_bus_name_owner(name: &'static str, timeout: Duration) -> bool {
    println!(
        "[GNOME] Waiting up to {}s for {} to appear on the session bus",
        timeout.as_secs(),
        name
    );

    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async move {
            let connection = match Connection::session().await {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to connect to session bus while waiting for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            let dbus = match zbus::fdo::DBusProxy::new(&connection).await {
                Ok(proxy) => proxy,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to create D-Bus proxy while waiting for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            if session_bus_name_has_owner(&dbus, name).await {
                println!("[GNOME] {} is already on the session bus", name);
                return true;
            }

            let mut owner_changes = match dbus
                .receive_name_owner_changed_with_args(&[(0, name)])
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!(
                        "[GNOME] Failed to subscribe to NameOwnerChanged for {}: {}",
                        name, error
                    );
                    return false;
                }
            };

            if session_bus_name_has_owner(&dbus, name).await {
                println!("[GNOME] {} appeared on the session bus during setup", name);
                return true;
            }

            let wait_for_owner = async {
                while let Some(signal) = owner_changes.next().await {
                    let args = match signal.args() {
                        Ok(args) => args,
                        Err(error) => {
                            eprintln!(
                                "[GNOME] Failed to decode NameOwnerChanged for {}: {}",
                                name, error
                            );
                            continue;
                        }
                    };

                    if args.new_owner().is_some() {
                        println!("[GNOME] {} appeared on the session bus", name);
                        return true;
                    }
                }

                false
            };

            match tokio::time::timeout(timeout, wait_for_owner).await {
                Ok(has_owner) => has_owner,
                Err(_) => {
                    eprintln!(
                        "[GNOME] Timed out after {}s waiting for {} on the session bus",
                        timeout.as_secs(),
                        name
                    );
                    false
                }
            }
        })
    })
}

async fn session_bus_name_has_owner(proxy: &zbus::fdo::DBusProxy<'_>, name: &str) -> bool {
    match proxy.name_has_owner(name.try_into().unwrap()).await {
        Ok(has_owner) => has_owner,
        Err(error) => {
            eprintln!(
                "[GNOME] Failed to check D-Bus ownership for {}: {}",
                name, error
            );
            false
        }
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

    if !status.shell_service_available {
        println!("[GNOME] Extension status: waiting for GNOME Shell D-Bus");
        return;
    }

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
    const GNOME_SHELL_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

    let mut status = gnome_extension_status();
    print_gnome_extension_status(&status);

    if !status.shell_service_available {
        if !wait_for_session_bus_name_owner(GNOME_SHELL_BUS_NAME, GNOME_SHELL_WAIT_TIMEOUT) {
            print_gnome_extension_status(&status);
            std::process::exit(1);
        }

        let mut elapsed_ms: u64 = 0;
        loop {
            status = gnome_extension_status();
            if status.shell_service_available {
                print_gnome_extension_status(&status);
                break;
            }

            if elapsed_ms >= GNOME_SHELL_WAIT_TIMEOUT.as_millis() as u64 {
                print_gnome_extension_status(&status);
                std::process::exit(1);
            }

            std::thread::sleep(Duration::from_millis(RETRY_INTERVAL_MS));
            elapsed_ms += RETRY_INTERVAL_MS;

            if elapsed_ms % 1_000 == 0 {
                println!(
                    "[GNOME] Waiting for GNOME Shell D-Bus interface... ({}ms/{}ms)",
                    elapsed_ms,
                    GNOME_SHELL_WAIT_TIMEOUT.as_millis()
                );
            }
        }
    }

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
    runtime_environment: Option<RuntimeEnvironmentBroadcaster>,
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
        let context = match &self.runtime_environment {
            Some(runtime_environment) => resolve_runtime_unpause_context(runtime_environment).await,
            None => UnpauseContext {
                env: self.env,
                connection: Some(self.focus_query_connection.clone()),
                is_kde6: self.is_kde6,
            },
        };
        unpause_daemon(
            context.env,
            context.connection,
            context.is_kde6,
            &self.pause_broadcaster,
            &self.handler,
            &self.status_broadcaster,
            &self.kanata,
            &self.runtime_handle,
            "via DBus",
        );
    }
}

async fn resolve_runtime_unpause_context(
    runtime_environment: &RuntimeEnvironmentBroadcaster,
) -> UnpauseContext {
    let env = runtime_environment.current();
    let connection = if environment_requires_focus_query_connection(env) {
        Some(Connection::session().await.unwrap_or_else(|error| {
            panic!(
                "[DBus] Failed to connect to session bus for unpause focus query: {}",
                error
            )
        }))
    } else {
        None
    };
    let is_kde6 = if env == Environment::Kde {
        let connection_ref = connection
            .as_ref()
            .expect("KDE runtime unpause context requires session connection");
        resolve_kde_runtime_query_mode_with_retry(connection_ref)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "[KDE] Failed to resolve runtime query mode for unpause: {}",
                    error
                )
            })
    } else {
        false
    };
    UnpauseContext {
        env,
        connection,
        is_kde6,
    }
}

async fn resolve_kde_runtime_query_mode_with_retry(
    connection: &Connection,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;

    for attempt in 0..KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS {
        match ensure_kde_scripting_ready(connection).await {
            Ok(()) => match resolve_kde_runtime_query_mode(connection).await {
                Ok(is_kde6) => return Ok(is_kde6),
                Err(error) => {
                    eprintln!(
                        "[KDE] Runtime query mode probe attempt {}/{} failed: {}",
                        attempt + 1,
                        KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS,
                        error
                    );
                    last_error = Some(error);
                }
            },
            Err(error) => {
                eprintln!(
                    "[KDE] Scripting readiness check attempt {}/{} failed: {}",
                    attempt + 1,
                    KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS,
                    error
                );
                last_error = Some(error);
            }
        }

        if attempt + 1 < KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS {
            tokio::time::sleep(KDE_RUNTIME_QUERY_MODE_RETRY_DELAY).await;
        }
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::other("[KDE] Runtime query mode probe failed without error").into()
    }))
}

async fn ensure_kde_scripting_ready(
    connection: &Connection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let has_owner = dbus
        .name_has_owner(KDE_KWIN_BUS_NAME.try_into().unwrap())
        .await?;
    if !has_owner {
        return Err(
            std::io::Error::other(format!("[KDE] {} is not owned", KDE_KWIN_BUS_NAME)).into(),
        );
    }

    if dbus
        .name_has_owner(KDE_KWIN_SCRIPTING_INTERFACE.try_into().unwrap())
        .await?
    {
        return Ok(());
    }

    let introspection = connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(DBUS_INTROSPECTABLE_INTERFACE),
            "Introspect",
            &(),
        )
        .await?;
    let introspection_xml: String = introspection.body().deserialize()?;
    if !introspection_xml.contains(KDE_KWIN_SCRIPTING_INTERFACE) {
        return Err(std::io::Error::other(format!(
            "[KDE] {} is not exported on {}",
            KDE_KWIN_SCRIPTING_INTERFACE, KDE_KWIN_SCRIPTING_PATH
        ))
        .into());
    }

    Ok(())
}

async fn resolve_kde_runtime_query_mode(
    connection: &Connection,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let probe_id = KDE_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    let script_path = kwin_query_probe_script_path(probe_id);
    fs::write(&script_path, "function kanataSwitcherProbe() {}\n")?;

    let load_result = connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "loadScript",
            &(&script_path,),
        )
        .await;
    let load_reply = match load_result {
        Ok(reply) => reply,
        Err(error) => {
            let _ = remove_kwin_probe_script_file(&script_path);
            return Err(Box::new(error));
        }
    };

    let script_num: i32 = match load_reply.body().deserialize() {
        Ok(script_num) => script_num,
        Err(error) => {
            let _ = unload_kwin_script_by_path(connection, &script_path).await;
            let _ = remove_kwin_probe_script_file(&script_path);
            return Err(Box::new(error));
        }
    };

    let kde6_path = format!("/Scripting/Script{}", script_num);
    let kde5_path = format!("/{}", script_num);
    let kde6_path_exists = kwin_object_path_exists(connection, kde6_path.as_str()).await;
    let kde5_path_exists = kwin_object_path_exists(connection, kde5_path.as_str()).await;

    unload_kwin_script_by_path(connection, &script_path).await?;
    remove_kwin_probe_script_file(&script_path)?;

    match (kde6_path_exists, kde5_path_exists) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (true, true) => Err(std::io::Error::other(
            "[KDE] Runtime query mode probe found both KDE5 and KDE6 script paths",
        )
        .into()),
        (false, false) => Err(std::io::Error::other(
            "[KDE] Runtime query mode probe found no known KWin script path",
        )
        .into()),
    }
}

async fn kwin_object_path_exists(connection: &Connection, object_path: &str) -> bool {
    connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            object_path,
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await
        .is_ok()
}

async fn unload_kwin_script_by_path(
    connection: &Connection,
    script_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "unloadScript",
            &(&script_path,),
        )
        .await?;
    Ok(())
}

fn remove_kwin_probe_script_file(
    script_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match fs::remove_file(script_path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Box::new(error)),
    }
}

fn environment_requires_focus_query_connection(env: Environment) -> bool {
    matches!(
        env,
        Environment::Gnome | Environment::Kde | Environment::Wayland | Environment::X11
    )
}

struct DbusServiceRegistration {
    _connection: Connection,
    status_signal_task: tokio::task::JoinHandle<()>,
    pause_signal_task: tokio::task::JoinHandle<()>,
}

impl Drop for DbusServiceRegistration {
    fn drop(&mut self) {
        self.status_signal_task.abort();
        self.pause_signal_task.abort();
    }
}

#[cfg(test)]
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
    effective_name: &str,
) -> Result<DbusServiceRegistration, DynError> {
    register_dbus_service_with_runtime_environment(
        connection,
        focus_query_connection,
        env,
        is_kde6,
        kanata,
        handler,
        status_broadcaster,
        restart_handle,
        pause_broadcaster,
        None,
        effective_name,
    )
    .await
}

async fn register_dbus_service_with_runtime_environment(
    connection: &Connection,
    focus_query_connection: Connection,
    env: Environment,
    is_kde6: bool,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: Option<RuntimeEnvironmentBroadcaster>,
    effective_name: &str,
) -> Result<DbusServiceRegistration, DynError> {
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
        runtime_environment,
    };

    connection.object_server().at(DBUS_PATH, service).await?;

    connection.request_name(effective_name).await?;

    let mut receiver = status_broadcaster.subscribe();
    let signal_emitter = SignalEmitter::new(connection, DBUS_PATH)?.into_owned();
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
    let status_signal_task = tokio::spawn(async move {
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
    let pause_signal_task = tokio::spawn(async move {
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

    Ok(DbusServiceRegistration {
        _connection: connection.clone(),
        status_signal_task,
        pause_signal_task,
    })
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
    let focus_query_connection = Connection::session().await?;
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

    let signal_connection = Connection::session().await?;
    let focus_signal_subscription = subscribe_to_gnome_focus_signal(
        &signal_connection,
        kanata.clone(),
        handler.clone(),
        status_broadcaster.clone(),
        pause_broadcaster.clone(),
    )
    .await?;

    println!(
        "[GNOME] Listening for FocusChanged signals from extension at {}",
        GNOME_FOCUS_OBJECT_PATH
    );
    let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
    drop(focus_signal_subscription);
    drop(signal_connection);
    Ok(outcome)
}

/// Guard owning the GNOME FocusChanged signal subscription task. Dropping the
/// guard aborts the listener (and releases the match rule when the connection
/// is dropped).
struct GnomeFocusSignalSubscription {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for GnomeFocusSignalSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn subscribe_to_gnome_focus_signal(
    connection: &Connection,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
) -> Result<GnomeFocusSignalSubscription, Box<dyn std::error::Error + Send + Sync>> {
    use zbus::MatchRule;
    let match_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(GNOME_SHELL_BUS_NAME)
        .map_err(|error| format!("Invalid sender match rule: {}", error))?
        .interface(GNOME_FOCUS_INTERFACE)
        .map_err(|error| format!("Invalid interface match rule: {}", error))?
        .path(GNOME_FOCUS_OBJECT_PATH)
        .map_err(|error| format!("Invalid path match rule: {}", error))?
        .member(GNOME_FOCUS_SIGNAL)
        .map_err(|error| format!("Invalid member match rule: {}", error))?
        .build();

    let mut stream = zbus::MessageStream::for_match_rule(match_rule, connection, None).await?;
    let task = tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    eprintln!("[GNOME] FocusChanged signal error: {}", error);
                    continue;
                }
            };
            let (window_class, window_title): (String, String) =
                match message.body().deserialize() {
                    Ok(payload) => payload,
                    Err(error) => {
                        eprintln!("[GNOME] FocusChanged decode error: {}", error);
                        continue;
                    }
                };
            let win = WindowInfo {
                class: window_class,
                title: window_title,
                is_native_terminal: false,
            };
            let default_layer = kanata.default_layer().await.unwrap_or_default();
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
    });
    Ok(GnomeFocusSignalSubscription { task })
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

/// Build the KWin focus-push script body. Targets the per-instance daemon
/// bus name so multiple daemons coexist on KDE with isolated push channels.
fn build_kde_focus_push_script(bus_name: &str, api: &str, active_window: &str) -> String {
    format!(
        r#"function notifyFocus(client) {{
  callDBus(
    "{bus}",
    "{path}",
    "{iface}",
    "WindowFocus",
    client ? (client.resourceClass || "") : "",
    client ? (client.caption || "") : ""
  );
}}
workspace.{api}.connect(notifyFocus);
notifyFocus(workspace.{active});
"#,
        bus = bus_name,
        path = DBUS_PATH,
        iface = DBUS_INTERFACE,
        api = api,
        active = active_window
    )
}

async fn run_kde(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let connection = Connection::session().await?;
    let focus_query_connection = Connection::session().await?;
    let runtime_handle = tokio::runtime::Handle::current();
    let is_kde6 = resolve_kde_runtime_query_mode_with_retry(&focus_query_connection).await?;

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
    let kwin_script = build_kde_focus_push_script(&effective_name, api, active_window);

    let script_path = kwin_runtime_script_path();
    fs::write(&script_path, &kwin_script)?;

    for _ in 0..5 {
        let result = connection
            .call_method(
                Some("org.kde.KWin"),
                KDE_KWIN_SCRIPTING_PATH,
                Some(KDE_KWIN_SCRIPTING_INTERFACE),
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
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "unloadScript",
            &(&script_path,),
        )
        .await;

    let load_result = connection
        .call_method(
            Some("org.kde.KWin"),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
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
        KDE_KWIN_SCRIPTING_INTERFACE
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

const DBUS_RECONNECT_DELAYS_MS: &[u64] = &[250, 1000, 2000];

fn dbus_reconnect_delay(attempt: usize) -> Duration {
    let index = attempt.min(DBUS_RECONNECT_DELAYS_MS.len() - 1);
    Duration::from_millis(DBUS_RECONNECT_DELAYS_MS[index])
}

async fn wait_for_dbus_reconnect_retry(
    reconnect_attempt: &mut usize,
    shutdown_receiver: &mut watch::Receiver<bool>,
    restart_receiver: &mut watch::Receiver<bool>,
    prefix: &str,
    error: String,
) {
    let delay = dbus_reconnect_delay(*reconnect_attempt);
    eprintln!("{}; retrying in {}ms: {}", prefix, delay.as_millis(), error);
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        _ = shutdown_receiver.changed() => {}
        _ = restart_receiver.changed() => {}
    }
    *reconnect_attempt += 1;
}

struct PersistentDbusServiceGuard {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for PersistentDbusServiceGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn start_persistent_dbus_service(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> PersistentDbusServiceGuard {
    start_persistent_dbus_service_with_connector(
        || async {
            Connection::session()
                .await
                .map_err(|error| -> DynError { Box::new(error) })
        },
        kanata,
        handler,
        status_broadcaster,
        restart_handle,
        pause_broadcaster,
        runtime_environment,
        shutdown_handle,
        effective_name,
    )
}

fn start_persistent_dbus_service_with_connector<C, CFut>(
    connector: C,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) -> PersistentDbusServiceGuard
where
    C: Fn() -> CFut + Send + Sync + 'static,
    CFut: std::future::Future<Output = Result<Connection, DynError>> + Send + 'static,
{
    let task = tokio::spawn(async move {
        run_persistent_dbus_service_with_connector(
            connector,
            kanata,
            handler,
            status_broadcaster,
            restart_handle,
            pause_broadcaster,
            runtime_environment,
            shutdown_handle,
            effective_name,
        )
        .await;
    });
    PersistentDbusServiceGuard { task }
}

async fn run_persistent_dbus_service_with_connector<C, CFut>(
    connector: C,
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    restart_handle: RestartHandle,
    pause_broadcaster: PauseBroadcaster,
    runtime_environment: RuntimeEnvironmentBroadcaster,
    shutdown_handle: ShutdownHandle,
    effective_name: String,
) where
    C: Fn() -> CFut + Send + Sync + 'static,
    CFut: std::future::Future<Output = Result<Connection, DynError>> + Send + 'static,
{
    let mut restart_receiver = restart_handle.subscribe();
    let mut shutdown_receiver = shutdown_handle.subscribe();
    let mut reconnect_attempt = 0usize;

    loop {
        if *shutdown_receiver.borrow() || *restart_receiver.borrow() {
            return;
        }

        let connection = match connector().await {
            Ok(connection) => {
                reconnect_attempt = 0;
                connection
            }
            Err(error) => {
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Session bus unavailable",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        let registration = match register_dbus_service_with_runtime_environment(
            &connection,
            connection.clone(),
            Environment::Unknown,
            false,
            kanata.clone(),
            handler.clone(),
            status_broadcaster.clone(),
            restart_handle.clone(),
            pause_broadcaster.clone(),
            Some(runtime_environment.clone()),
            &effective_name,
        )
        .await
        {
            Ok(registration) => {
                println!("[DBus] Control service registered as {}", effective_name);
                reconnect_attempt = 0;
                registration
            }
            Err(error) => {
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to register control service",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        let proxy = match zbus::fdo::DBusProxy::new(&connection).await {
            Ok(proxy) => proxy,
            Err(error) => {
                drop(registration);
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to create DBus proxy for name-loss monitoring",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };
        let mut name_lost = match proxy
            .receive_name_lost_with_args(&[(0, effective_name.as_str())])
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                drop(registration);
                wait_for_dbus_reconnect_retry(
                    &mut reconnect_attempt,
                    &mut shutdown_receiver,
                    &mut restart_receiver,
                    "[DBus] Failed to subscribe to NameLost",
                    error.to_string(),
                )
                .await;
                continue;
            }
        };

        tokio::select! {
            _ = shutdown_receiver.changed() => {
                drop(registration);
            }
            _ = restart_receiver.changed() => {
                drop(registration);
            }
            signal = name_lost.next() => {
                match signal {
                    Some(signal) => {
                        match signal.args() {
                            Ok(args) => {
                                eprintln!(
                                    "[DBus] Lost well-known name {}; re-registering",
                                    args.name()
                                );
                            }
                            Err(error) => {
                                eprintln!(
                                    "[DBus] Failed to decode NameLost signal; re-registering: {}",
                                    error
                                );
                            }
                        }
                    }
                    None => {
                        eprintln!("[DBus] NameLost stream terminated; re-registering");
                    }
                }
                drop(registration);
            }
        }
    }
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
        let dispatch = match args.dbus_suffix.as_deref() {
            Some(suffix) => ControlDispatch::Unicast {
                bus_name: effective_dbus_name(suffix),
            },
            None => ControlDispatch::Broadcast,
        };
        send_control_command(command, dispatch).await?;
        return Ok(RunOutcome::Exit);
    }
    let effective_name = effective_dbus_name(&resolve_dbus_suffix(
        args.dbus_suffix.as_deref(),
        &args.host,
        args.port,
    )?);
    println!("[DBus] Using bus name: {}", effective_name);

    let install_gnome_extension = resolve_install_gnome_extension(&matches);

    let detected_env = detect_environment();
    println!("[Init] Detected environment: {}", detected_env.as_str());

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

    let focus_handler = Arc::new(Mutex::new(FocusHandler::new(
        config.rules.clone(),
        config.native_terminal_rule.clone(),
        quiet_focus,
    )));

    let lifecycle_provider = LifecycleProvider::build(detected_env).await;
    if !lifecycle_provider.is_continuous() && detected_env == Environment::Unknown {
        eprintln!("[Error] Could not detect display environment");
        eprintln!("[Error] login1 unavailable and no startup graphical environment detected");
        std::process::exit(1);
    }

    // Create shutdown guard - will switch to default layer when dropped
    let _shutdown_guard = ShutdownGuard::new(kanata.clone());
    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    let _persistent_dbus_service_guard = start_persistent_dbus_service(
        kanata.clone(),
        focus_handler.clone(),
        status_broadcaster.clone(),
        restart_handle.clone(),
        pause_broadcaster.clone(),
        runtime_environment.clone(),
        shutdown_handle.clone(),
        effective_name.clone(),
    );

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

    let enable_indicator = !args.no_indicator;
    if args.no_indicator {
        println!("[SNI] Indicator disabled via --no-indicator");
    }

    let _sni_guard = if enable_indicator {
        SniGuard::runtime_managed(
            runtime_environment.clone(),
            runtime_handle.clone(),
            kanata.clone(),
            focus_handler.clone(),
            status_broadcaster.clone(),
            pause_broadcaster.clone(),
            restart_handle.clone(),
            shutdown_handle.clone(),
            args.indicator_focus_only,
            effective_name.clone(),
        )
    } else {
        SniGuard::disabled()
    };

    let backend_context = BackendContext {
        kanata,
        handler: focus_handler,
        status_broadcaster,
        restart_handle: restart_handle.clone(),
        pause_broadcaster,
        runtime_environment,
        install_gnome_extension,
        gnome_setup_completed: Arc::new(AtomicBool::new(false)),
        gnome_setup_hook: Arc::new(setup_gnome_extension),
        effective_dbus_name: effective_name,
    };

    run_lifecycle_supervisor(
        lifecycle_provider,
        backend_context,
        restart_handle,
        shutdown_handle,
    )
    .await
}

// === Tests ===

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;
