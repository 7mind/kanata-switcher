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
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot, watch};
use uuid::Uuid;
use zbus::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

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
mod control;
mod pause;
mod focus_pipeline;
mod lifecycle;
mod display_override;
mod supervisor;
mod backends;

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
use control::{ControlCommand, ControlDispatch};
use control::client::*;
use pause::*;
use focus_pipeline::*;
use lifecycle::*;
use lifecycle::logind::*;
use lifecycle::startup::*;
use display_override::*;
use supervisor::*;
use backends::*;
use backends::wayland::*;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::{constants::*, errors::*, environ::*, dbus_naming::*, config::*, focus::*, args::*, autostart::*, broadcasters::*, kanata::*, control::*, control::client::*, pause::*, focus_pipeline::*, lifecycle::*, lifecycle::logind::*, lifecycle::startup::*, display_override::*, supervisor::*, supervisor::capabilities::*, backends::*, backends::gnome::*, backends::kde::*, backends::kde::script::*, backends::kde::probe::*, backends::wayland::*};


// === SNI Indicator ===

pub(crate) const SNI_DEFAULT_SHOW_FOCUS_ONLY: bool = true;
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

pub(crate) struct SniSettingsStore {
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

pub(crate) async fn session_bus_name_has_owner(proxy: &zbus::fdo::DBusProxy<'_>, name: &str) -> bool {
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
