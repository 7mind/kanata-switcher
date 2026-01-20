use super::*;
use clap::Parser;
use zbus::Message;
use proptest::prelude::*;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn with_test_timeout<F, T>(future: F) -> T
where
    F: Future<Output = T>,
{
    tokio::time::timeout(TEST_TIMEOUT, future)
        .await
        .expect("test timeout")
}

fn win(class: &str, title: &str) -> WindowInfo {
    WindowInfo {
        class: class.to_string(),
        title: title.to_string(),
        is_native_terminal: false,
    }
}

fn rule(class: Option<&str>, title: Option<&str>, layer: Option<&str>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: title.map(String::from),
        on_native_terminal: None,
        layer: layer.map(String::from),
        virtual_key: None,
        raw_vk_action: None,
        fallthrough: false,
    }
}

fn rule_vk(class: Option<&str>, virtual_key: &str) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        on_native_terminal: None,
        layer: None,
        virtual_key: Some(virtual_key.to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }
}

fn rule_raw_vk(class: Option<&str>, raw_vk_action: Vec<(&str, &str)>) -> Rule {
    Rule {
        class: class.map(String::from),
        title: None,
        on_native_terminal: None,
        layer: None,
        virtual_key: None,
        raw_vk_action: Some(
            raw_vk_action
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ),
        fallthrough: false,
    }
}

fn rule_with_fallthrough(mut r: Rule) -> Rule {
    r.fallthrough = true;
    r
}

/// Helper to check if actions contain a specific action
fn has_action(actions: &FocusActions, action: &FocusAction) -> bool {
    actions.actions.contains(action)
}

/// Helper to get all actions of a specific type
fn get_layers(actions: &FocusActions) -> Vec<String> {
    actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::ChangeLayer(l) = a {
                Some(l.clone())
            } else {
                None
            }
        })
        .collect()
}

fn get_raw_vk_actions(actions: &FocusActions) -> Vec<(String, String)> {
    actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::RawVkAction(n, act) = a {
                Some((n.clone(), act.clone()))
            } else {
                None
            }
        })
        .collect()
}

// === Flow Tests ===

#[test]
fn test_basic_layer_match() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("browser".to_string())]
    );
}

#[test]
fn test_no_match_uses_default() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("default".to_string())]
    );
}

#[test]
fn test_same_window_no_action() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab1"), "default");
    assert_eq!(actions, None);
}

#[test]
fn test_same_rule_different_window_no_action() {
    let rules = vec![rule(None, None, Some("global"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", "tab1"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("global".to_string())]
    );

    let actions = handler.handle(&win("kitty", "tab2"), "default");
    assert_eq!(actions, None);
}

#[test]
fn test_same_rule_different_window_no_action_with_vk_and_raw() {
    let rules = vec![Rule {
        class: None,
        title: None,
        on_native_terminal: None,
        layer: Some("global".to_string()),
        virtual_key: Some("vk_global".to_string()),
        raw_vk_action: Some(vec![("vk_raw".to_string(), "Tap".to_string())]),
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", "tab1"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("global".to_string()),
            FocusAction::PressVk("vk_global".to_string()),
            FocusAction::RawVkAction("vk_raw".to_string(), "Tap".to_string()),
        ]
    );

    let actions = handler.handle(&win("kitty", "tab2"), "default");
    assert_eq!(actions, None);
}
#[test]
fn test_title_change_triggers_action() {
    let rules = vec![
        rule(Some("kitty"), Some("vim"), Some("vim")),
        rule(Some("kitty"), None, Some("terminal")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", "bash"), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["terminal".to_string()]);

    let actions = handler.handle(&win("kitty", "vim"), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["vim".to_string()]);
}

#[test]
fn test_unfocus_releases_vk_and_switches_to_default() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: None,
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("", ""), "default").unwrap();

    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ReleaseVk("vk_browser".to_string()),
            FocusAction::ChangeLayer("default".to_string()),
        ]
    );
    assert_eq!(actions.new_managed_vks, Vec::<String>::new());
}

#[test]
fn test_native_terminal_rule_applies_actions() {
    let rules = vec![rule(Some("kitty"), None, Some("terminal"))];
    let native_rule = Some(NativeTerminalRule {
        layer: "tty".to_string(),
        virtual_key: Some("vk_tty".to_string()),
        raw_vk_action: vec![("vk_notify".to_string(), "Tap".to_string())],
    });
    let mut handler = FocusHandler::new(rules, native_rule, true);

    let actions = handler
        .handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: true,
            },
            "default",
        )
        .unwrap();

    assert!(has_action(
        &actions,
        &FocusAction::ChangeLayer("tty".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_tty".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::RawVkAction("vk_notify".to_string(), "Tap".to_string())
    ));
}

#[test]
fn test_autostart_passthrough_args_skip_oneshot() {
    let matches = Args::command().get_matches_from([
        "kanata-switcher",
        "--install-autostart",
        "-p",
        "12000",
        "--quiet-focus",
        "--no-indicator",
    ]);
    let args = Args::from_arg_matches(&matches).unwrap();
    let exec_args = autostart_passthrough_args(&matches, &args);
    assert_eq!(
        exec_args,
        vec![
            "-p".to_string(),
            "12000".to_string(),
            "--quiet-focus".to_string(),
            "--no-indicator".to_string()
        ]
    );
}

#[test]
fn test_autostart_passthrough_args_indicator_focus_only() {
    let matches = Args::command().get_matches_from([
        "kanata-switcher",
        "--install-autostart",
        "--indicator-focus-only",
        "false",
    ]);
    let args = Args::from_arg_matches(&matches).unwrap();
    let exec_args = autostart_passthrough_args(&matches, &args);
    assert_eq!(
        exec_args,
        vec!["--indicator-focus-only".to_string(), "false".to_string()]
    );
}

#[test]
fn test_autostart_desktop_content_escapes_exec() {
    let exec_path = Path::new("/tmp/kanata switcher");
    let exec_args = vec![
        "--quiet-focus".to_string(),
        "-c".to_string(),
        "/tmp/config%file.json".to_string(),
    ];
    let content = build_autostart_desktop_content(exec_path, &exec_args);
    assert!(content.contains("Type=Application\n"));
    assert!(content.contains("Name=Kanata Switcher\n"));
    assert!(content.contains("X-GNOME-Autostart-enabled=true\n"));
    assert!(content.contains(
        "Exec=\"/tmp/kanata switcher\" \"--quiet-focus\" \"-c\" \"/tmp/config%%file.json\"\n"
    ));
    assert!(content.contains("TryExec=\"/tmp/kanata switcher\"\n"));
}

#[test]
fn test_native_terminal_without_rule_uses_default() {
    let rules = vec![rule(Some("kitty"), None, Some("terminal"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler
        .handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: true,
            },
            "default",
        )
        .unwrap();

    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("default".to_string())]
    );
}

#[test]
fn test_control_command_restart() {
    let args = Args::parse_from(["kanata-switcher", "--restart"]);
    assert_eq!(
        resolve_control_command(&args),
        Some(ControlCommand::Restart)
    );
}

#[test]
fn test_control_command_pause() {
    let args = Args::parse_from(["kanata-switcher", "--pause"]);
    assert_eq!(resolve_control_command(&args), Some(ControlCommand::Pause));
}

#[test]
fn test_control_command_unpause() {
    let args = Args::parse_from(["kanata-switcher", "--unpause"]);
    assert_eq!(
        resolve_control_command(&args),
        Some(ControlCommand::Unpause)
    );
}

#[test]
fn test_control_command_none() {
    let args = Args::parse_from(["kanata-switcher"]);
    assert_eq!(resolve_control_command(&args), None);
}

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
    assert_eq!(SniIndicator::format_virtual_keys(&keys), "∞");
}

#[derive(Clone, Default)]
struct MockSniControlCounts {
    restart: usize,
    pause: usize,
    unpause: usize,
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
}

#[derive(Default)]
struct MockGsettingsState {
    get_calls: usize,
    set_calls: Vec<bool>,
}

struct MockGsettingsBackend {
    state: Arc<Mutex<MockGsettingsState>>,
    get_results: Arc<Mutex<Vec<Result<bool, String>>>>,
    set_results: Arc<Mutex<Vec<Result<(), String>>>>,
}

impl MockGsettingsBackend {
    fn new(
        state: Arc<Mutex<MockGsettingsState>>,
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

impl GsettingsBackend for MockGsettingsBackend {
    fn get_bool(
        &self,
        _schema_dir: Option<&Path>,
        _schema: &str,
        _key: &str,
    ) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        state.get_calls += 1;
        self.next_get_result()
    }

    fn set_bool(
        &self,
        _schema_dir: Option<&Path>,
        _schema: &str,
        _key: &str,
        value: bool,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.set_calls.push(value);
        self.next_set_result()
    }
}

fn mock_gsettings_backend_sequence(
    get_results: Vec<Result<bool, String>>,
    set_results: Vec<Result<(), String>>,
) -> (Box<dyn GsettingsBackend>, Arc<Mutex<MockGsettingsState>>) {
    let state = Arc::new(Mutex::new(MockGsettingsState::default()));
    let backend = MockGsettingsBackend::new(state.clone(), get_results, set_results);
    (Box::new(backend), state)
}

fn mock_gsettings_backend(
    get_result: Result<bool, String>,
    set_result: Result<(), String>,
) -> (Box<dyn GsettingsBackend>, Arc<Mutex<MockGsettingsState>>) {
    mock_gsettings_backend_sequence(vec![get_result], vec![set_result])
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
fn test_sni_settings_store_reads_from_gsettings() {
    let (backend, state) = mock_gsettings_backend(Ok(false), Ok(()));
    let mut store = SniSettingsStore::with_backend(Vec::new(), backend);
    let value = store.read_focus_only();
    assert_eq!(value, Some(false));
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_settings_store_fallback_schema_dir() {
    let (backend, state) = mock_gsettings_backend_sequence(
        vec![
            Err("No such schema".to_string()),
            Ok(true),
        ],
        vec![Ok(())],
    );
    let mut store = SniSettingsStore::with_backend(vec![PathBuf::from("/schema")], backend);
    let value = store.read_focus_only();
    assert_eq!(value, Some(true));
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 2);
}

#[test]
fn test_sni_settings_store_read_error_disables_write() {
    let (backend, state) = mock_gsettings_backend(
        Err("No such file or directory".to_string()),
        Ok(()),
    );
    let mut store = SniSettingsStore::with_backend(Vec::new(), backend);
    let value = store.read_focus_only();
    assert_eq!(value, None);
    store.write_focus_only(true);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_settings_store_schema_error_allows_write_attempt() {
    let (backend, state) = mock_gsettings_backend(
        Err("No such schema".to_string()),
        Err("No such schema".to_string()),
    );
    let mut store = SniSettingsStore::with_backend(Vec::new(), backend);
    let value = store.read_focus_only();
    assert_eq!(value, None);
    store.write_focus_only(true);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 1);
    assert_eq!(state.set_calls, vec![true]);
}

#[test]
fn test_resolve_sni_focus_only_override_skips_gsettings() {
    let (backend, state) = mock_gsettings_backend(Ok(false), Ok(()));
    let mut store = SniSettingsStore::with_backend(Vec::new(), backend);
    let value = resolve_sni_focus_only(Some(TrayFocusOnly::True), &mut store);
    assert!(value);
    let state = state.lock().unwrap();
    assert_eq!(state.get_calls, 0);
    assert!(state.set_calls.is_empty());
}

#[test]
fn test_sni_toggle_persists_to_gsettings() {
    let (backend, state) = mock_gsettings_backend(Ok(true), Ok(()));
    let store = SniSettingsStore::with_backend(Vec::new(), backend);
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
            _ => {}
        }
    }

    assert!(found_pause);
    assert!(found_restart);
    let counts = control_counts.counts();
    assert_eq!(counts.pause, 1);
    assert_eq!(counts.restart, 1);
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

#[tokio::test]
async fn test_update_status_for_focus_updates_snapshot() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    let win = win("firefox", "");
    let actions = update_status_for_focus(
        &handler,
        &status_broadcaster,
        &win,
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_some());

    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "browser");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_update_status_for_focus_unknown_layer_uses_default() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let kanata = KanataClient::new(
        "127.0.0.1",
        10000,
        Some("default".to_string()),
        true,
        status_broadcaster.clone(),
    );

    {
        let mut inner = kanata
            .inner
            .try_lock()
            .expect("Expected KanataClient lock");
        inner.known_layers = vec!["default".to_string()];
    }

    let win = win("firefox", "");
    let actions = update_status_for_focus(
        &handler,
        &status_broadcaster,
        &win,
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_some());

    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "default");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_handle_focus_event_ignored_when_paused_no_status_change() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    pause_broadcaster.set_paused(true);
    let win = win("firefox", "");
    let actions = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &win,
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_none());
    let snapshot = status_broadcaster.snapshot();
    assert!(snapshot.layer.is_empty());

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
    assert!(actions.is_some());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "browser");
    assert_eq!(snapshot.layer_source, LayerSource::Focus);
}

#[tokio::test]
async fn test_handle_focus_event_unfocus_paused_does_not_switch_layer() {
    let rules = vec![rule(Some("firefox"), None, Some("browser"))];
    let handler = Arc::new(Mutex::new(FocusHandler::new(rules, None, true)));
    let status_broadcaster = StatusBroadcaster::new();
    let pause_broadcaster = PauseBroadcaster::new();
    let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

    status_broadcaster.update_layer("current".to_string(), LayerSource::External);
    pause_broadcaster.set_paused(true);

    let actions = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &WindowInfo::default(),
        &kanata,
        "default",
    )
    .await;
    assert!(actions.is_none());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "current");
    assert_eq!(snapshot.layer_source, LayerSource::External);
}

#[test]
fn test_paused_status_resets_virtual_keys_and_source() {
    let status_broadcaster = StatusBroadcaster::new();
    status_broadcaster.update_layer("external".to_string(), LayerSource::External);
    status_broadcaster.update_virtual_keys(vec!["vk_browser".to_string()]);
    status_broadcaster.set_paused_status("base".to_string());
    let snapshot = status_broadcaster.snapshot();
    assert_eq!(snapshot.layer, "base");
    assert!(snapshot.virtual_keys.is_empty());
    assert_eq!(snapshot.layer_source, LayerSource::External);
}

#[test]
fn test_virtual_key_press_on_focus() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_browser".to_string())
    ));
    assert!(
        !actions
            .actions
            .iter()
            .any(|a| matches!(a, FocusAction::ReleaseVk(_)))
    );
    assert_eq!(actions.new_managed_vks, vec!["vk_browser".to_string()]);
}

#[test]
fn test_virtual_key_release_on_switch() {
    let rules = vec![
        rule_vk(Some("firefox"), "vk_browser"),
        rule_vk(Some("kitty"), "vk_terminal"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", ""), "default");
    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Release comes before Press in the action list
    let release_idx = actions
        .actions
        .iter()
        .position(|a| matches!(a, FocusAction::ReleaseVk(_)));
    let press_idx = actions
        .actions
        .iter()
        .position(|a| matches!(a, FocusAction::PressVk(_)));
    assert!(release_idx.unwrap() < press_idx.unwrap());

    assert!(has_action(
        &actions,
        &FocusAction::ReleaseVk("vk_browser".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk_terminal".to_string())
    ));
}

#[test]
fn test_virtual_key_no_change_no_press() {
    let rules = vec![rule_vk(Some("firefox"), "vk_browser")];
    let mut handler = FocusHandler::new(rules, None, true);

    handler.handle(&win("firefox", "tab1"), "default");
    let actions = handler.handle(&win("firefox", "tab2"), "default");

    // Window changed but VK is the same - no VK actions (VK already held)
    assert!(
        actions.is_none()
            || !actions
                .as_ref()
                .unwrap()
                .actions
                .iter()
                .any(|a| matches!(a, FocusAction::PressVk(_) | FocusAction::ReleaseVk(_)))
    );
}

#[test]
fn test_partial_vk_set_change_only_releases_removed() {
    // Two rules with fallthrough: vk1 and vk2 are both held
    // Then switch to a window that only matches vk2 - only vk1 should be released
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: Some("both".to_string()),
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus window that matches both rules - both VKs pressed
    let actions = handler.handle(&win("app", "both"), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string()]
    );

    // Focus window that only matches second rule - only vk1 should be released, vk2 stays held
    let actions = handler.handle(&win("app", "other"), "default").unwrap();
    assert!(has_action(
        &actions,
        &FocusAction::ReleaseVk("vk1".to_string())
    ));
    assert!(!has_action(
        &actions,
        &FocusAction::ReleaseVk("vk2".to_string())
    ));
    assert!(!has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    )); // vk2 already held
    assert_eq!(actions.new_managed_vks, vec!["vk2".to_string()]);
}

#[test]
fn test_unfocus_releases_multiple_vks_in_reverse_order() {
    // Multiple VKs held should be released in reverse order (bottom-to-top)
    let rules = vec![
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: None,
            fallthrough: true,
        },
        Rule {
            class: Some("app".to_string()),
            title: None,
            on_native_terminal: None,
            layer: None,
            virtual_key: Some("vk3".to_string()),
            raw_vk_action: None,
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    // Focus window - all three VKs pressed in order
    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]
    );

    // Unfocus - all VKs should be released in reverse order (vk3, vk2, vk1)
    let actions = handler.handle(&win("", ""), "default").unwrap();
    let release_actions: Vec<_> = actions
        .actions
        .iter()
        .filter_map(|a| {
            if let FocusAction::ReleaseVk(vk) = a {
                Some(vk.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        release_actions,
        vec!["vk3".to_string(), "vk2".to_string(), "vk1".to_string()]
    );
}

#[test]
fn test_raw_vk_action_fires_on_focus() {
    let rules = vec![rule_raw_vk(
        Some("firefox"),
        vec![("vk1", "Tap"), ("vk2", "Toggle")],
    )];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("vk1".to_string(), "Tap".to_string()),
            ("vk2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_fallthrough_collects_all_layers() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("kitty"), None, Some("layer1"))),
        rule(Some("kitty"), None, Some("layer2")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both layers should be in the action list, in order
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer2".to_string()]
    );
}

#[test]
fn test_fallthrough_add_remove_rule_only_new_actions() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("base"))),
        rule(Some("app"), Some("special"), Some("special")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", "special"), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec!["base".to_string(), "special".to_string()]
    );

    let actions = handler.handle(&win("app", "other"), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![FocusAction::ChangeLayer("base".to_string())]
    );
}

#[test]
fn test_fallthrough_collects_all_raw_vk_actions() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(Some("kitty"), vec![("vk1", "Press")])),
        rule_raw_vk(Some("kitty"), vec![("vk2", "Tap")]),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("vk1".to_string(), "Press".to_string()),
            ("vk2".to_string(), "Tap".to_string()),
        ]
    );
}

#[test]
fn test_fallthrough_all_vks_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_vk(Some("kitty"), "vk2"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // Both vk1 and vk2 should be pressed (all matched VKs are held)
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string()]
    );
}

#[test]
fn test_fallthrough_multiple_vks_all_pressed_and_held() {
    let rules = vec![
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk1")),
        rule_with_fallthrough(rule_vk(Some("kitty"), "vk2")),
        rule_vk(Some("kitty"), "vk3"),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();
    // All three VKs should be pressed and held
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk1".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk2".to_string())
    ));
    assert!(has_action(
        &actions,
        &FocusAction::PressVk("vk3".to_string())
    ));
    assert_eq!(
        actions.new_managed_vks,
        vec!["vk1".to_string(), "vk2".to_string(), "vk3".to_string()]
    );
}

#[test]
fn test_fallthrough_action_order_preserved() {
    // Test that actions from each rule are in order: layer, vk, raw_vk
    let rules = vec![
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer1".to_string()),
            virtual_key: Some("vk1".to_string()),
            raw_vk_action: Some(vec![("raw1".to_string(), "Tap".to_string())]),
            fallthrough: true,
        },
        Rule {
            class: Some("kitty".to_string()),
            title: None,
            on_native_terminal: None,
            layer: Some("layer2".to_string()),
            virtual_key: Some("vk2".to_string()),
            raw_vk_action: Some(vec![("raw2".to_string(), "Toggle".to_string())]),
            fallthrough: false,
        },
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("kitty", ""), "default").unwrap();

    // Expected order: layer1, PressVk(vk1), raw1, layer2, PressVk(vk2), raw2
    // All matched VKs are pressed (not tapped)
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("layer1".to_string()),
            FocusAction::PressVk("vk1".to_string()),
            FocusAction::RawVkAction("raw1".to_string(), "Tap".to_string()),
            FocusAction::ChangeLayer("layer2".to_string()),
            FocusAction::PressVk("vk2".to_string()),
            FocusAction::RawVkAction("raw2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_combined_virtual_key_and_raw_vk_action() {
    let rules = vec![Rule {
        class: Some("firefox".to_string()),
        title: None,
        on_native_terminal: None,
        layer: Some("browser".to_string()),
        virtual_key: Some("vk_browser".to_string()),
        raw_vk_action: Some(vec![("vk_notify".to_string(), "Tap".to_string())]),
        fallthrough: false,
    }];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("firefox", ""), "default").unwrap();
    assert_eq!(
        actions.actions,
        vec![
            FocusAction::ChangeLayer("browser".to_string()),
            FocusAction::PressVk("vk_browser".to_string()),
            FocusAction::RawVkAction("vk_notify".to_string(), "Tap".to_string()),
        ]
    );
}

#[test]
fn test_wildcard_pattern() {
    let rules = vec![rule(Some("*"), None, Some("any"))];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("anything", ""), "default").unwrap();
    assert_eq!(get_layers(&actions), vec!["any".to_string()]);
}

#[test]
fn test_regex_pattern() {
    let rules = vec![rule(Some("^(firefox|chrome)$"), None, Some("browser"))];
    let mut handler = FocusHandler::new(rules, None, true);

    assert_eq!(
        get_layers(&handler.handle(&win("firefox", ""), "default").unwrap()),
        vec!["browser".to_string()]
    );
    assert_eq!(handler.handle(&win("chrome", ""), "default"), None);
    assert_eq!(
        get_layers(&handler.handle(&win("chromium", ""), "default").unwrap()),
        vec!["default".to_string()]
    );
}

#[test]
fn test_three_rules_fallthrough_all_layers_execute() {
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("app"), None, Some("layer2"))),
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec![
            "layer1".to_string(),
            "layer2".to_string(),
            "layer3".to_string(),
        ]
    );
}

#[test]
fn test_multiple_raw_vk_actions_per_rule_all_execute() {
    let rules = vec![
        rule_with_fallthrough(rule_raw_vk(
            Some("app"),
            vec![("a1", "Press"), ("a2", "Release")],
        )),
        rule_raw_vk(Some("app"), vec![("b1", "Tap"), ("b2", "Toggle")]),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_raw_vk_actions(&actions),
        vec![
            ("a1".to_string(), "Press".to_string()),
            ("a2".to_string(), "Release".to_string()),
            ("b1".to_string(), "Tap".to_string()),
            ("b2".to_string(), "Toggle".to_string()),
        ]
    );
}

#[test]
fn test_non_fallthrough_stops_chain() {
    // First rule matches but has fallthrough=false, should stop chain
    let rules = vec![
        rule(Some("app"), None, Some("layer1")), // fallthrough=false
        rule(Some("app"), None, Some("layer2")), // would match but shouldn't be reached
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // Only layer1 should be collected
    assert_eq!(get_layers(&actions), vec!["layer1".to_string()]);
}

#[test]
fn test_fallthrough_stops_at_non_fallthrough() {
    // First two rules have fallthrough, third doesn't - chain stops at third
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("app"), None, Some("layer2"))),
        rule(Some("app"), None, Some("layer3")), // fallthrough=false, stops here
        rule(Some("app"), None, Some("layer4")), // should not be reached
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    assert_eq!(
        get_layers(&actions),
        vec![
            "layer1".to_string(),
            "layer2".to_string(),
            "layer3".to_string(),
        ]
    );
    // layer4 should NOT be present
}

#[test]
fn test_non_matching_rules_skipped_in_fallthrough() {
    // Rules that don't match should be skipped even with fallthrough
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule_with_fallthrough(rule(Some("other"), None, Some("layer2"))), // doesn't match
        rule(Some("app"), None, Some("layer3")),
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer2 should be skipped because "other" doesn't match "app"
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer3".to_string(),]
    );
}

#[test]
fn test_non_matching_non_fallthrough_rule_does_not_stop_chain() {
    // A non-matching rule with fallthrough=false should NOT stop the chain
    // (only matching rules can stop the chain)
    let rules = vec![
        rule_with_fallthrough(rule(Some("app"), None, Some("layer1"))),
        rule(Some("other"), None, Some("layer2")), // doesn't match, fallthrough=false
        rule(Some("app"), None, Some("layer3")),   // should still be reached
    ];
    let mut handler = FocusHandler::new(rules, None, true);

    let actions = handler.handle(&win("app", ""), "default").unwrap();
    // layer1 and layer3 should be collected; layer2 skipped (doesn't match)
    // The non-matching rule's fallthrough=false should NOT stop the chain
    assert_eq!(
        get_layers(&actions),
        vec!["layer1".to_string(), "layer3".to_string(),]
    );
}

// === Property Tests ===

fn arb_class() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("firefox".to_string()),
        Just("kitty".to_string()),
        Just("code".to_string()),
        Just("alacritty".to_string()),
        Just("".to_string()),
        "[a-z]{1,10}".prop_map(String::from),
    ]
}

fn arb_nonempty_class() -> impl Strategy<Value = String> {
    arb_class().prop_filter("non-empty class", |class| !class.is_empty())
}

fn arb_title() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        Just("vim".to_string()),
        Just("bash".to_string()),
        "[a-zA-Z0-9 ]{0,20}".prop_map(String::from),
    ]
}

fn arb_layer() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("default".to_string()),
        Just("browser".to_string()),
        Just("terminal".to_string()),
        Just("vim".to_string()),
        "[a-z]{1,8}".prop_map(String::from),
    ]
}

fn arb_vk_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("vk1".to_string()),
        Just("vk2".to_string()),
        Just("vk_browser".to_string()),
        Just("vk_terminal".to_string()),
    ]
}

fn arb_vk_action() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("Press".to_string()),
        Just("Release".to_string()),
        Just("Tap".to_string()),
        Just("Toggle".to_string()),
    ]
}

fn arb_rule() -> impl Strategy<Value = Rule> {
    (
        prop::option::of(arb_class()),
        prop::option::of(arb_title()),
        prop::option::of(arb_layer()),
        prop::option::of(arb_vk_name()),
        prop::option::of(prop::collection::vec(
            (arb_vk_name(), arb_vk_action()),
            0..3,
        )),
        any::<bool>(),
    )
        .prop_map(|(class, title, layer, vk, raw_vk, fallthrough)| Rule {
            class,
            title,
            on_native_terminal: None,
            layer,
            virtual_key: vk,
            raw_vk_action: raw_vk,
            fallthrough,
        })
}

fn arb_window() -> impl Strategy<Value = WindowInfo> {
    (arb_class(), arb_title()).prop_map(|(class, title)| WindowInfo {
        class,
        title,
        is_native_terminal: false,
    })
}

proptest! {
    #[test]
    fn prop_managed_vks_consistent(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 1..10),
    ) {
        let mut handler = FocusHandler::new(rules, None, true);

        for win in &windows {
            let _ = handler.handle(win, "default");
            // Just verify the handler state is consistent (Vec is always valid)
            prop_assert!(handler.current_virtual_keys.len() <= 10); // sanity bound
        }
    }

    #[test]
    fn prop_releases_before_presses(
        rules in prop::collection::vec(arb_rule(), 1..5),
        windows in prop::collection::vec(arb_window(), 2..10),
    ) {
        let mut handler = FocusHandler::new(rules, None, true);

        for win in &windows {
            if let Some(actions) = handler.handle(win, "default") {
                // All ReleaseVk actions should come before any PressVk actions
                let first_press_idx = actions.actions.iter().position(|a| matches!(a, FocusAction::PressVk(_)));
                let last_release_idx = actions.actions.iter().rposition(|a| matches!(a, FocusAction::ReleaseVk(_)));

                if let (Some(press_idx), Some(release_idx)) = (first_press_idx, last_release_idx) {
                    prop_assert!(release_idx < press_idx, "All releases should come before presses");
                }
            }
        }
    }

    #[test]
    fn prop_unfocus_releases_all_vks(
        rules in prop::collection::vec(arb_rule(), 1..5),
        win in arb_window(),
    ) {
        let mut handler = FocusHandler::new(rules, None, true);

        // Focus a window first
        let _ = handler.handle(&win, "default");
        let vks_before = handler.current_virtual_keys.clone();

        // Unfocus (empty class and title)
        let actions = handler.handle(
            &WindowInfo {
                class: String::new(),
                title: String::new(),
                is_native_terminal: false,
            },
            "default",
        );

        // All previously active VKs must be released
        if !vks_before.is_empty() {
            prop_assert!(actions.is_some());
            let actions = actions.unwrap();
            for old_vk in &vks_before {
                prop_assert!(has_action(&actions, &FocusAction::ReleaseVk(old_vk.clone())));
            }
        }
        // After unfocus, no VKs should be active
        prop_assert!(handler.current_virtual_keys.is_empty());
    }

    #[test]
    fn prop_fallthrough_collects_all_raw_vk(
        base_class in arb_nonempty_class(),
        raw_vk1 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
        raw_vk2 in prop::collection::vec((arb_vk_name(), arb_vk_action()), 0..2),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk1.is_empty() { None } else { Some(raw_vk1.clone()) },
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: None,
                raw_vk_action: if raw_vk2.is_empty() { None } else { Some(raw_vk2.clone()) },
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            let expected: Vec<_> = raw_vk1.into_iter().chain(raw_vk2).collect();
            prop_assert_eq!(get_raw_vk_actions(&actions), expected);
        }
    }

    #[test]
    fn prop_fallthrough_collects_all_layers(
        base_class in arb_nonempty_class(),
        layer1 in arb_layer(),
        layer2 in arb_layer(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: Some(layer1.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: Some(layer2.clone()),
                virtual_key: None,
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both layers should be collected
            prop_assert_eq!(get_layers(&actions), vec![layer1, layer2]);
        }
    }

    #[test]
    fn prop_all_matched_vks_pressed_and_held(
        base_class in arb_nonempty_class(),
        vk1 in arb_vk_name(),
        vk2 in arb_vk_name(),
    ) {
        let rules = vec![
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: Some(vk1.clone()),
                raw_vk_action: None,
                fallthrough: true,
            },
            Rule {
                class: Some(base_class.clone()),
                title: None,
                on_native_terminal: None,
                layer: None,
                virtual_key: Some(vk2.clone()),
                raw_vk_action: None,
                fallthrough: false,
            },
        ];

        let mut handler = FocusHandler::new(rules, None, true);
        let win = WindowInfo {
            class: base_class,
            title: String::new(),
            is_native_terminal: false,
        };

        if let Some(actions) = handler.handle(&win, "default") {
            // Both vk1 and vk2 should be pressed (all matched VKs are held)
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk1.clone())));
            prop_assert!(has_action(&actions, &FocusAction::PressVk(vk2.clone())));
            // new_managed_vks should contain both
            prop_assert_eq!(actions.new_managed_vks, vec![vk1, vk2]);
        }
    }
}

// === GNOME Extension State Parsing Tests ===

#[test]
fn test_gnome_extension_state_enabled_f64() {
    // GNOME Shell returns state as f64, 1.0 = ENABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert(
        "state".to_string(),
        OwnedValue::try_from(Value::F64(1.0)).unwrap(),
    );

    let status = parse_gnome_extension_state(&body);
    assert!(status.active, "state=1.0 should be active");
    assert!(status.enabled, "state=1.0 should be enabled");
    assert!(status.installed, "D-Bus response means installed");
}

#[test]
fn test_gnome_extension_state_disabled_f64() {
    // State 2.0 = DISABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert(
        "state".to_string(),
        OwnedValue::try_from(Value::F64(2.0)).unwrap(),
    );

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "state=2.0 should not be active");
    assert!(!status.enabled, "state=2.0 should not be enabled");
}

#[test]
fn test_gnome_extension_state_missing() {
    // No state field - should default to not enabled
    let body = HashMap::new();

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "missing state should not be active");
    assert!(!status.enabled, "missing state should not be enabled");
}

#[tokio::test]
async fn test_logind_monitor_startup_failure_is_non_fatal() {
    with_test_timeout(async {
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

        let started = start_logind_session_monitor_best_effort(
            Environment::Wayland,
            None,
            false,
            handler,
            status_broadcaster.clone(),
            pause_broadcaster,
            kanata,
            |_env, _session, _is_kde6, _handler, _status, _pause, _kanata| async {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "logind unavailable").into())
            },
        )
        .await;

        assert!(!started);
    })
    .await;
}

#[tokio::test]
async fn test_logind_monitor_startup_success_returns_true() {
    with_test_timeout(async {
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let kanata = KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone());

        let started = start_logind_session_monitor_best_effort(
            Environment::Wayland,
            None,
            false,
            handler,
            status_broadcaster.clone(),
            pause_broadcaster,
            kanata,
            |_env, _session, _is_kde6, _handler, _status, _pause, _kanata| async { Ok(()) },
        )
        .await;

        assert!(started);
    })
    .await;
}

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_returns_restart() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let wait_future = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle);
        let trigger_future = async {
            tokio::task::yield_now().await;
            restart_handle.request();
        };
        let (outcome, _) = tokio::join!(wait_future, trigger_future);
        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_returns_exit() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        shutdown_handle.request();

        let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_shutdown_wins() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        shutdown_handle.request();
        restart_handle.request();

        let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[test]
fn test_logind_no_session_error_detection() {
    use zbus::names::OwnedErrorName;
    use zbus::{Error as ZbusError, Message};

    let name = OwnedErrorName::try_from(LOGIND_ERROR_NO_SESSION_FOR_PID).unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSessionByPID")
        .unwrap()
        .build(&())
        .unwrap();
    let error = ZbusError::MethodError(name, Some("no session".to_string()), reply);

    assert!(is_logind_no_session_error(&error));
}

#[test]
fn test_logind_no_session_error_detection_false() {
    use zbus::names::OwnedErrorName;
    use zbus::{Error as ZbusError, Message};

    let name = OwnedErrorName::try_from("org.freedesktop.login1.NoSuchSession").unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&())
        .unwrap();
    let error = ZbusError::MethodError(name, Some("no session".to_string()), reply);

    assert!(!is_logind_no_session_error(&error));
}

#[test]
fn test_logind_empty_object_path_detection() {
    let empty = OwnedObjectPath::try_from(LOGIND_EMPTY_OBJECT_PATH).unwrap();
    let non_empty = OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();

    assert!(is_logind_empty_object_path(&empty));
    assert!(!is_logind_empty_object_path(&non_empty));
}

#[test]
fn test_parse_logind_object_path_value() {
    use zbus::zvariant::ObjectPath;

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let value = OwnedValue::from(path);
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_parse_logind_object_path_structure_single_field() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let structure = StructureBuilder::new().add_field(path).build().unwrap();
    let value = OwnedValue::try_from(structure).unwrap();
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_parse_logind_object_path_string() {
    use zbus::zvariant::Str;

    let value = OwnedValue::from(Str::from("/org/freedesktop/login1/session/_1"));
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_object_path() {
    use zbus::zvariant::ObjectPath;

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&path)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_structure() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let structure = StructureBuilder::new().add_field(path).build().unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&structure)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_structure_multi_field() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_311").unwrap();
    let structure = StructureBuilder::new()
        .add_field("11")
        .add_field(path)
        .build()
        .unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&structure)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_311");
}

#[test]
fn test_decode_logind_object_path_reply_variant() {
    use zbus::zvariant::{ObjectPath, Value};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let value = OwnedValue::try_from(Value::from(path)).unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&value)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}
