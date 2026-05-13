use super::*;

#[test]
fn test_sni_format_layer_letter() {
    assert_eq!(SniIndicator::format_layer_letter("base"), "B");
    assert_eq!(SniIndicator::format_layer_letter(""), "?");
    assert_eq!(SniIndicator::format_layer_letter("  "), "?");
}

#[test]
fn test_sni_format_virtual_keys() {
    assert_eq!(SniIndicator::format_virtual_keys(&[]), "");
    assert_eq!(
        SniIndicator::format_virtual_keys(&[String::from("vk_media")]),
        "V"
    );
    assert_eq!(
        SniIndicator::format_virtual_keys(&[String::from("a"), String::from("b")]),
        "2"
    );
    let keys = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    assert_eq!(SniIndicator::format_virtual_keys(&keys), "9+");
}

fn sni_buffer_has_layer_pixels(buffer: &[u8]) -> bool {
    buffer.chunks_exact(4).any(|chunk| {
        let [alpha, red, green, blue] = chunk else {
            return false;
        };
        *alpha > 0 && *red > 0 && *red == *green && *green == *blue
    })
}

fn sni_buffer_has_vk_pixels(buffer: &[u8]) -> bool {
    buffer.chunks_exact(4).any(|chunk| {
        let [alpha, red, green, blue] = chunk else {
            return false;
        };
        *alpha > 0 && *red == 0 && *green > 0 && *blue > 0
    })
}

#[test]
fn test_sni_icon_color_layers_and_vks() {
    let icon = SniIndicator::render_icon("A", "B");
    assert!(sni_buffer_has_layer_pixels(&icon.data));
    assert!(sni_buffer_has_vk_pixels(&icon.data));
}

#[test]
fn test_sni_icon_color_layer_only() {
    let icon = SniIndicator::render_icon("A", "");
    assert!(sni_buffer_has_layer_pixels(&icon.data));
    assert!(!sni_buffer_has_vk_pixels(&icon.data));
}

#[derive(Clone, Default)]
struct MockSniControlCounts {
    restart: usize,
    pause: usize,
    unpause: usize,
    quit: usize,
}

#[derive(Clone)]
struct MockSniControl {
    counts: Arc<Mutex<MockSniControlCounts>>,
}

impl MockSniControl {
    fn new() -> Self {
        Self {
            counts: Arc::new(Mutex::new(MockSniControlCounts::default())),
        }
    }

    fn counts(&self) -> MockSniControlCounts {
        self.counts.lock().unwrap().clone()
    }
}

impl SniControlOps for MockSniControl {
    fn restart(&self) {
        self.counts.lock().unwrap().restart += 1;
    }

    fn pause(&self) {
        self.counts.lock().unwrap().pause += 1;
    }

    fn unpause(&self) {
        self.counts.lock().unwrap().unpause += 1;
    }

    fn quit(&self) {
        self.counts.lock().unwrap().quit += 1;
    }
}

#[derive(Default)]
struct MockDconfState {
    get_calls: usize,
    set_calls: Vec<bool>,
}

struct MockDconfBackend {
    state: Arc<Mutex<MockDconfState>>,
    get_results: Arc<Mutex<Vec<Result<bool, String>>>>,
    set_results: Arc<Mutex<Vec<Result<(), String>>>>,
}

impl MockDconfBackend {
    fn new(
        state: Arc<Mutex<MockDconfState>>,
        get_results: Vec<Result<bool, String>>,
        set_results: Vec<Result<(), String>>,
    ) -> Self {
        Self {
            state,
            get_results: Arc::new(Mutex::new(get_results)),
            set_results: Arc::new(Mutex::new(set_results)),
        }
    }

    fn next_get_result(&self) -> Result<bool, String> {
        let mut results = self.get_results.lock().unwrap();
        if results.is_empty() {
            return Err("no mock get results left".to_string());
        }
        results.remove(0)
    }

    fn next_set_result(&self) -> Result<(), String> {
        let mut results = self.set_results.lock().unwrap();
        if results.is_empty() {
            return Err("no mock set results left".to_string());
        }
        results.remove(0)
    }
}

impl DconfBackend for MockDconfBackend {
    fn get_bool(&self, _key: &str) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        state.get_calls += 1;
        self.next_get_result()
    }

    fn set_bool(&self, _key: &str, value: bool) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.set_calls.push(value);
        self.next_set_result()
    }
}

fn mock_dconf_backend_sequence(
    get_results: Vec<Result<bool, String>>,
    set_results: Vec<Result<(), String>>,
) -> (Box<dyn DconfBackend>, Arc<Mutex<MockDconfState>>) {
    let state = Arc::new(Mutex::new(MockDconfState::default()));
    let backend = MockDconfBackend::new(state.clone(), get_results, set_results);
    (Box::new(backend), state)
}

fn mock_dconf_backend(
    get_result: Result<bool, String>,
    set_result: Result<(), String>,
) -> (Box<dyn DconfBackend>, Arc<Mutex<MockDconfState>>) {
    mock_dconf_backend_sequence(vec![get_result], vec![set_result])
}

#[test]
fn test_sni_indicator_state_focus_only() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let mut state = SniIndicatorState::new(initial.clone(), SNI_DEFAULT_SHOW_FOCUS_ONLY);
    assert_eq!(state.display_status().layer, "base");

    let focus_status = StatusSnapshot {
        layer: "browser".to_string(),
        virtual_keys: vec!["vk_browser".to_string()],
        layer_source: LayerSource::Focus,
    };
    state.update_status(focus_status.clone());
    assert_eq!(state.display_status().layer, "browser");

    state.toggle_focus_only();
    assert_eq!(state.display_status().layer, "browser");

    let external_status = StatusSnapshot {
        layer: "external".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    state.update_status(external_status.clone());
    assert_eq!(state.display_status().layer, "external");

    state.toggle_focus_only();
    assert_eq!(state.display_status().layer, "browser");

    state.set_paused(true);
    assert_eq!(state.display_status().layer, "external");
}

#[test]
fn test_sni_indicator_state_initial_focus_only_false() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let mut state = SniIndicatorState::new(initial.clone(), false);

    let focus_status = StatusSnapshot {
        layer: "browser".to_string(),
        virtual_keys: vec!["vk_browser".to_string()],
        layer_source: LayerSource::Focus,
    };
    state.update_status(focus_status);

    let external_status = StatusSnapshot {
        layer: "external".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    state.update_status(external_status);

    assert_eq!(state.display_status().layer, "external");
}

#[test]
fn test_sni_settings_store_reads_from_dconf() {
    let (backend, state) = mock_dconf_backend(Ok(false), Ok(()));
    let mut store = SniSettingsStore::with_backend(backend);
    let value = store.read_focus_only();
    assert_eq!(value, Some(false));
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_settings_store_writes_to_dconf() {
    let (backend, state) = mock_dconf_backend(Ok(true), Ok(()));
    let mut store = SniSettingsStore::with_backend(backend);
    store.write_focus_only(false);
    let state = state.lock().unwrap();
    assert_eq!(state.set_calls, vec![false]);
}

#[test]
fn test_sni_settings_store_read_error_disables_write() {
    let (backend, state) = mock_dconf_backend(Err("No such file or directory".to_string()), Ok(()));
    let mut store = SniSettingsStore::with_backend(backend);
    let value = store.read_focus_only();
    assert_eq!(value, None);
    store.write_focus_only(true);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_settings_store_key_not_set_allows_write() {
    let (backend, state) = mock_dconf_backend(Err("key not set".to_string()), Ok(()));
    let mut store = SniSettingsStore::with_backend(backend);
    let value = store.read_focus_only();
    assert_eq!(value, None);
    store.write_focus_only(true);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert_eq!(state.set_calls, vec![true]);
}

#[test]
fn test_resolve_sni_focus_only_override_skips_dconf() {
    let (backend, state) = mock_dconf_backend(Ok(false), Ok(()));
    let mut store = SniSettingsStore::with_backend(backend);
    let value = resolve_sni_focus_only(Some(TrayFocusOnly::True), &mut store);
    assert!(value);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 0);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_toggle_persists_to_dconf() {
    let (backend, state) = mock_dconf_backend(Ok(true), Ok(()));
    let store = SniSettingsStore::with_backend(backend);
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let (menu_refresh, _menu_receiver) = MenuRefresh::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, true),
        control: Arc::new(control),
        settings: store,
        menu_refresh,
    };

    indicator.toggle_focus_only();
    let state = state.lock().unwrap();
    assert_eq!(state.set_calls, vec![false]);
}

#[test]
fn test_sni_toggle_sends_menu_refresh() {
    let (menu_refresh, receiver) = MenuRefresh::new();
    let receiver = receiver;
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, SNI_DEFAULT_SHOW_FOCUS_ONLY),
        control: Arc::new(control),
        settings: SniSettingsStore::disabled(),
        menu_refresh,
    };

    indicator.toggle_focus_only();
    assert_eq!(*receiver.borrow(), 1);
}

#[test]
fn test_sni_menu_actions_dispatch_control() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let control_counts = control.clone();
    let (menu_refresh, _menu_receiver) = MenuRefresh::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, SNI_DEFAULT_SHOW_FOCUS_ONLY),
        control: Arc::new(control),
        settings: SniSettingsStore::disabled(),
        menu_refresh,
    };

    let menu = indicator.menu();
    let mut found_pause = false;
    let mut found_restart = false;
    let mut found_quit = false;
    for item in menu {
        match item {
            MenuItem::Checkmark(check) if check.label == "Pause" => {
                found_pause = true;
                (check.activate)(&mut indicator);
            }
            MenuItem::Standard(standard) if standard.label == "Restart" => {
                found_restart = true;
                (standard.activate)(&mut indicator);
            }
            MenuItem::Standard(standard) if standard.label == "Quit" => {
                found_quit = true;
                (standard.activate)(&mut indicator);
            }
            _ => {}
        }
    }

    assert!(found_pause);
    assert!(found_restart);
    assert!(found_quit);
    let counts = control_counts.counts();
    assert_eq!(counts.pause, 1);
    assert_eq!(counts.restart, 1);
    assert_eq!(counts.quit, 1);
}

#[test]
fn test_sni_menu_toggle_affects_display() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let (menu_refresh, _menu_receiver) = MenuRefresh::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, SNI_DEFAULT_SHOW_FOCUS_ONLY),
        control: Arc::new(control),
        settings: SniSettingsStore::disabled(),
        menu_refresh,
    };

    let focus_status = StatusSnapshot {
        layer: "browser".to_string(),
        virtual_keys: vec!["vk_browser".to_string()],
        layer_source: LayerSource::Focus,
    };
    indicator.update_status(focus_status);

    let (layer_text, _) = indicator.display_strings();
    assert_eq!(layer_text, "B");

    let external_status = StatusSnapshot {
        layer: "external".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    indicator.update_status(external_status);

    indicator.toggle_focus_only();
    let (layer_text, vk_text) = indicator.display_strings();
    assert_eq!(layer_text, "E");
    assert!(vk_text.is_empty());

    indicator.toggle_focus_only();
    let (layer_text, vk_text) = indicator.display_strings();
    assert_eq!(layer_text, "B");
    assert_eq!(vk_text, "V");

    let tooltip = indicator.tooltip_text();
    assert!(tooltip.contains("Layer:"));
}

#[test]
fn test_sni_tooltip_includes_virtual_keys() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let (menu_refresh, _menu_receiver) = MenuRefresh::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, SNI_DEFAULT_SHOW_FOCUS_ONLY),
        control: Arc::new(control),
        settings: SniSettingsStore::disabled(),
        menu_refresh,
    };

    let focus_status = StatusSnapshot {
        layer: "browser".to_string(),
        virtual_keys: vec!["vk_browser".to_string(), "vk_media".to_string()],
        layer_source: LayerSource::Focus,
    };
    indicator.update_status(focus_status);
    let tooltip = indicator.tooltip_text();
    assert!(tooltip.contains("Layer: browser"));
    assert!(tooltip.contains("vk_browser"));
    assert!(tooltip.contains("vk_media"));
}

#[test]
fn test_sni_title_text_is_single_line() {
    let initial = StatusSnapshot {
        layer: "base".to_string(),
        virtual_keys: Vec::new(),
        layer_source: LayerSource::External,
    };
    let control = MockSniControl::new();
    let (menu_refresh, _menu_receiver) = MenuRefresh::new();
    let mut indicator = SniIndicator {
        state: SniIndicatorState::new(initial, SNI_DEFAULT_SHOW_FOCUS_ONLY),
        control: Arc::new(control),
        settings: SniSettingsStore::disabled(),
        menu_refresh,
    };

    let focus_status = StatusSnapshot {
        layer: "browser".to_string(),
        virtual_keys: vec!["vk_browser".to_string(), "vk_media".to_string()],
        layer_source: LayerSource::Focus,
    };
    indicator.update_status(focus_status);

    let title = indicator.title_text();
    assert!(title == "Kanata Switcher");
}
