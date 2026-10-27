use std::sync::{Arc, Mutex};
use std::thread;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
pub(crate) use ksni::{Icon as SniIcon, MenuItem, Status as SniStatus, ToolTip, Tray, TrayService};
use ksni::menu::{CheckmarkItem, StandardItem};
use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};
use crate::broadcasters::{StatusBroadcaster, PauseBroadcaster};
use crate::args::TrayFocusOnly;
use crate::focus_pipeline::resolve_sni_focus_only;
use super::settings::SniSettingsStore;
use super::state::{SniIndicatorState, MenuRefresh};
use super::control_ops::SniControlOps;

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

pub(crate) struct SniIndicator {
    pub(crate) state: SniIndicatorState,
    pub(crate) control: Arc<dyn SniControlOps>,
    pub(crate) settings: SniSettingsStore,
    pub(crate) menu_refresh: MenuRefresh,
}

impl SniIndicator {
    pub(crate) fn update_status(&mut self, snapshot: crate::broadcasters::StatusSnapshot) {
        self.state.update_status(snapshot);
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.state.set_paused(paused);
    }

    pub(crate) fn toggle_focus_only(&mut self) {
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

    pub(crate) fn format_layer_letter(layer_name: &str) -> String {
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

    pub(crate) fn format_virtual_keys(virtual_keys: &[String]) -> String {
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

    pub(crate) fn render_icon(layer_text: &str, vk_text: &str) -> SniIcon {
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

    pub(crate) fn display_strings(&self) -> (String, String) {
        let status = self.state.display_status();
        let layer_text = Self::format_layer_letter(&status.layer);
        let vk_text = Self::format_virtual_keys(&status.virtual_keys);
        (layer_text, vk_text)
    }

    pub(crate) fn tooltip_text(&self) -> String {
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

    pub(crate) fn title_text(&self) -> String {
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


pub(crate) struct SniIndicatorRuntimeHandle {
    pub(crate) handle: ksni::Handle<SniIndicator>,
    pub(crate) status_watch_task: tokio::task::JoinHandle<()>,
    pub(crate) pause_watch_task: tokio::task::JoinHandle<()>,
    pub(crate) menu_watch_task: tokio::task::JoinHandle<()>,
}

impl SniIndicatorRuntimeHandle {
    pub(crate) fn shutdown(&self) {
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
pub(crate) static ACTIVE_SNI_WATCHER_TASKS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) struct SniWatcherTaskGuard;

#[cfg(test)]
impl SniWatcherTaskGuard {
    pub(crate) fn new() -> Self {
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
pub(crate) fn sni_watcher_task_count() -> usize {
    ACTIVE_SNI_WATCHER_TASKS.load(Ordering::SeqCst)
}

pub(crate) fn start_sni_indicator(
    control: super::SniControl,
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
