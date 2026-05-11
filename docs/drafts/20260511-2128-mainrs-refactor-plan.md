# Refactor Plan: src/daemon/main.rs → trait-based multi-file architecture

Branch surveyed: `persistent-daemon`. Target: `src/daemon/main.rs` at 7983 lines. Sibling files: `src/daemon/tests.rs` (4864), `src/daemon/integration_tests.rs` (7176) — both read internals via `use super::*`.

---

## 1. Goals and non-goals

### Operational success criteria
- **Per-file LOC budget**: no source file in `src/daemon/` exceeds ~1200 LOC after the refactor (excluding the two test files, which are out of scope).
- **`main.rs` final size**: ≤ 400 LOC, containing only `#[tokio::main] async fn main`, `async fn run_once`, signal handler wiring, the top-level `mod` declarations, and the `mod tests; mod integration_tests;` attachments.
- **Diff locality**: a typical bug fix (e.g. tweaking KDE script generation, or adding an SNI menu item) touches at most two `.rs` files.
- **Trait seam count**: 3–4 traits introduced or formalized at module boundaries (`FocusBackend`, plus the two already-extant `DconfBackend` and `SniControlOps` made into the consumed type at call sites). `LifecycleProvider` stays an enum — see §4.
- **No public-surface change**: identical CLI flags, identical DBus interface/path/signal/method names, identical log lines that integration tests grep for, identical config schema, identical on-disk file paths.
- **Tests green at every step**: every PR ends with `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `nix build`, and `nix run .#test` all passing.

### Non-goals
- No behaviour changes, no new features, no log-message rewording, no error-handling changes.
- No new dependencies, no version bumps.
- No splitting of `tests.rs` or `integration_tests.rs` (out of scope; their `use super::*` will continue to work).
- No `Cargo.toml [[bin]].path` change. `main.rs` stays the bin entrypoint.
- No "improving" code spotted in passing. Smells noticed during a PR get logged in §8, not fixed in flight.
- No reorganising `src/gnome-extension/*` (JS) or `src/protocols/*.xml`.
- No converting the `LifecycleProvider` enum into a trait — single production implementation per variant, dispatch is trivial, the indirection would not pay.

---

## 2. Current structure inventory

Line ranges below from `/tmp/main_structure.txt` and direct reads. Numbers are item-start lines.

| Cluster | Lines | Contents (selected names) |
|---|---|---|
| Imports + COSMIC scanner modules | 1–89 | `use` block, `mod cosmic_workspace`, `mod cosmic_toplevel` (each with `wayland_scanner::generate_interfaces!` / `generate_client_code!` referencing `src/protocols/cosmic-*.xml`) |
| Constants | 91–125 | `GNOME_EXTENSION_UUID`, `DBUS_BASE_NAME`, `DAEMON_BUS_NAME_PREFIX`, `DBUS_PATH`, `DBUS_INTERFACE`, `GNOME_FOCUS_*`, `KDE_QUERY_*`, `LOGIND_*`, `KDE_KWIN_*`, `KDE_RUNTIME_QUERY_MODE_*` |
| DBus naming | 127–230 | `DbusSuffixError`, `sanitize_dbus_suffix`, `derive_default_dbus_suffix`, `resolve_dbus_suffix`, `effective_dbus_name`, `is_daemon_bus_name` |
| Control commands (client side) | 232–278 | `ControlCommand`, `TrayFocusOnly` |
| CLI args | 280–402 | `Args` (clap derive), `parse_dbus_suffix_arg`, `resolve_install_gnome_extension`, `resolve_control_command` |
| Autostart | 403–559 | `resolve_binary_path`, `autostart_dir`, `autostart_desktop_path`, `escape_desktop_exec_arg`, `build_autostart_desktop_content`, `autostart_passthrough_args`, `install_autostart_desktop`, `uninstall_autostart_desktop` |
| Control dispatch (client side) | 561–696 | `ControlDispatch`, `send_control_command`, `send_control_command_with_connection`, `BroadcastEntryReport`, `BroadcastReport`, `enumerate_daemon_names`, `send_control_command_broadcast` |
| Config + matching | 697–917 | `Rule`, `NativeTerminalRule`, `ConfigEntry`, `Config`, `WindowInfo`, `load_config`, `match_pattern` |
| Focus engine | 919–1244 | `FocusAction`, `FocusActions`, `FocusHandler` (impl ~290 lines) |
| Broadcasters / handles | 1246–1464 | `StatusSnapshot`, `LayerSource`, `StatusBroadcaster`, `RestartHandle`, `PauseBroadcaster`, `RuntimeEnvironmentBroadcaster`, `ShutdownHandle`, `wait_for_restart_or_shutdown` |
| SNI settings + state + transitions | 1466–1739 | `DconfBackend` (trait), `ShellDconfBackend`, `SniSettingsStore`, `MenuRefresh`, `SniIndicatorState`, `UnpauseContext`, `SniLocalControl`, `SniDbusControl`, `SniControl`, `SniControlMode`, `SniRuntimeTransitionPlan`, `sni_control_mode_for_environment`, `local_sni_unpause_context`, `plan_sni_runtime_transition`, `SniRuntimeWakeReason`, `wait_for_sni_runtime_wake_with_delay` |
| SNI control ops + indicator | 1740–2118 | `SniControlOps` (trait), `impl SniControlOps for SniControl`, `SniIndicator` (with `impl Tray`) |
| Focus apply/handle helpers | 2119–2213 | `resolve_sni_focus_only`, `execute_focus_actions`, `extract_focus_layer`, `update_status_for_focus`, `handle_focus_event`, `native_terminal_window` |
| Wayland/X11 query helpers | 2214–2308 | `RawFdWatcher`, `resolve_wayland_socket_path`, `connect_wayland_with_display_override`, `query_wayland_active_window`, `wayland_query_count`, `query_x11_active_window` |
| KDE script generation + probe | 2309–2493 | `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, `KdeFocusQueryService`, `kwin_script_object_path`, `load_kwin_script`, `build_kde_query_script`, `query_kde_focus` |
| GNOME query + env dispatch | 2495–2569 | `query_gnome_focus`, `query_focus_for_env`, `apply_focus_for_env` |
| Logind session/display | 2571–2865 | `resolve_logind_session_path`, `is_logind_no_session_error`, `LogindSessionPathResolutionError`, `is_logind_empty_object_path`, `parse_logind_object_path`, `parse_logind_object_path_from_structure`, `decode_logind_object_path_reply`, `logind_object_path_from_value`, `resolve_logind_display_session_path`, `LogindDisplayPathChange`, `decode_logind_display_path_change`, `snapshot_no_session`, `LogindDisplayChangeAction`, `apply_logind_display_change`, `wait_for_logind_display_session_path` + `type DynError = ...` |
| Lifecycle providers | 2867–3202 | `StartupSnapshotProvider`, `LogindLifecycleProvider`, `verify_logind_lifecycle_monitor_prerequisites`, `validate_active_logind_session_type`, `fail_fast_lifecycle_monitor`, `expect_some_or_fail_fast`, `expect_or_fail_fast`, `monitor_logind_lifecycle`, `open_logind_session_monitor`, `decode_logind_lifecycle_snapshot_change` |
| Lifecycle supervisor | 3203–4145 | `LifecycleProvider` (enum), `BackendExit`, `BackendContext`, `BackendHandle`, `runtime_target_*` helpers, `detect_desktop_capabilities`, `resolve_runtime_target_for_snapshot`, `run_*_backend_task` (gnome/kde/wayland/x11/linux_console), `ensure_runtime_gnome_extension_setup`, display-override resolution helpers (`display_override_expected_session_type`, `is_valid_wayland_display_override`, `normalize_display_override`, `resolve_display_override_from_logind`, `display_override_backend_kind_for_environment`, `TestFocusQueryDisplayOverrideGuard`, `display_override_test_slot`, `set/resolve_test_focus_query_display_override`, `resolve_display_override_for_backend_kind`, `resolve_display_override_for_environment`), `start_backend`, `SupervisorState`, `transition_runtime_target` (+ `_with_starter`), `stop_current_backend`, `run_lifecycle_supervisor` (+ `_with_starter`, `_with_starter_and_resolver`), `wait_for_wayland_capability_recheck`, `wait_for_backend_completion_signal`, `poll_finished_backend_outcome` |
| Pause / unpause | 4147–4244 | `pause_daemon`, `unpause_daemon`, test-only `record/take_unpause_request_environment_for_test` |
| Kanata protocol messages | 4246–4317 | `ChangeLayerMsg`, `ChangeLayerPayload`, `LayerChangeMsg`, `LayerChangePayload`, `RequestLayerNamesMsg`, `RequestLayerNamesPayload`, `ActOnFakeKeyMsg`, `ActOnFakeKeyPayload`, `LayerNamesMsg`, `LayerNamesPayload`, `RequestFakeKeyNamesMsg`, `RequestFakeKeyNamesPayload`, `FakeKeyNamesMsg`, `FakeKeyNamesPayload` |
| Kanata client | 4319–4866 | `KanataClientInner`, `pub struct KanataClient`, `impl KanataClient` (~510 lines: connect, reconnect-with-queue, layer_names, fake_key_names, send VK action, default-layer resolution, status broadcast wiring) |
| ShutdownGuard | 4868–4885 | `ShutdownGuard` with `Drop` that resets to default layer |
| Environment + capabilities | 4887–5086 | `pub enum Environment`, `RunOutcome`, `SessionKind`, `DesktopFlavor`, `BackendKind`, `RuntimeTarget`, `DesktopCapabilities`, `LifecycleSnapshot`, `session_type_to_session_kind`, `session_type_indicates_native_terminal`, `resolve_desktop_flavor`, `resolve_runtime_target`, `runtime_target_from_wayland_startup_session_type_hint`, `target_requires_session_bus`, `startup_environment_to_snapshot`, `Environment::as_str/...`, `detect_environment` |
| Wayland backend | 5088–5459 | `ToplevelWindow`, `WaylandState`, `impl WaylandState`, all `Dispatch` impls (wl_registry, ZwlrForeignToplevelManagerV1/HandleV1, ZcosmicToplevelInfoV1/HandleV1, ZcosmicWorkspaceManagerV1/GroupHandleV1/HandleV1, wl_output), `WaylandProtocol`, `async fn run_wayland` |
| X11 backend | 5460–5660 | `X11State`, `impl X11State`, `async fn run_x11` |
| SNI start + control build + handles | 5661–5990 | `start_sni_indicator`, `build_sni_control_for_mode`, `SniIndicatorRuntimeHandle`, test-only `ACTIVE_SNI_WATCHER_TASKS`/`SniWatcherTaskGuard`/`sni_watcher_task_count`, `SniGuard` |
| dconf helpers | 5992–6033 | `dconf_get_bool`, `dconf_set_bool`, `is_dconf_unavailable` |
| GNOME extension lifecycle | 6034–6757 | constants (`GNOME_EXTENSION_SRC_PATH`, schema paths), `gnome_ext_file!` macro, `EMBEDDED_*` `include_str!` consts (feature-gated), `get_gnome_extension_fs_path`, `gnome_extension_fs_exists`, `compile_gnome_schemas`, `write_embedded_extension_to_dir`, `GnomeDetectionMethod`, `GnomeExtensionStatus`, `gnome_state_name`, `parse_gnome_extension_state`, `GnomeDbusProbeResult`, `is_dbus_service_unavailable`, `gnome_extension_dbus_probe[_with_connection]`, `gnome_extension_status`, `wait_for_session_bus_name_owner`, `session_bus_name_has_owner`, `print_gnome_extension_install_instructions`, `pack_and_install_from_dir`, `install_gnome_extension`, `enable_gnome_extension`, `ensure_gnome_extension`, `print_gnome_extension_status`, `setup_gnome_extension` |
| DBus control server | 6758–7224 | `DbusWindowFocusService` (zbus `#[interface]` impl with `WindowFocus`, `GetStatus`, `GetPaused`, `Pause`, `Unpause`, `Restart`, signal emitters), `resolve_runtime_unpause_context`, `resolve_kde_runtime_query_mode_with_retry`, `ensure_kde_scripting_ready`, `resolve_kde_runtime_query_mode`, `kwin_object_path_exists`, `unload_kwin_script_by_path`, `remove_kwin_probe_script_file`, `environment_requires_focus_query_connection`, `DbusServiceRegistration`, `register_dbus_service`, `register_dbus_service_with_runtime_environment` |
| GNOME backend | 7225–7341 | `async fn run_gnome`, `GnomeFocusSignalSubscription`, `subscribe_to_gnome_focus_signal` |
| KDE backend | 7342–7563 | `KwinScriptGuard`, `build_kde_focus_push_script`, `async fn run_kde` |
| Persistent DBus reconnector | 7565–7800 | `dbus_reconnect_delay`, `wait_for_dbus_reconnect_retry`, `PersistentDbusServiceGuard`, `start_persistent_dbus_service`, `start_persistent_dbus_service_with_connector`, `run_persistent_dbus_service_with_connector` |
| Entrypoint | 7801–7976 | `main`, `run_once`, `mod tests`, `mod integration_tests` |

### Cross-cutting types referenced from many clusters
- `Environment`, `RuntimeTarget`, `BackendKind`, `SessionKind`, `LifecycleSnapshot`, `DesktopCapabilities`, `DesktopFlavor` — referenced from CLI parsing (no), lifecycle (yes), supervisor (yes), SNI (mode resolution), backends (capabilities), DBus server (paused-state semantics). Should land in a foundational module early.
- `WindowInfo`, `Rule`, `Config`, `FocusActions`, `FocusAction`, `FocusHandler` — referenced from every focus path, from DBus server (`WindowFocus`), KDE/GNOME/Wayland/X11 backends.
- `KanataClient` — referenced from every backend and from supervisor, SNI, DBus server, pause/unpause.
- `StatusBroadcaster`, `PauseBroadcaster`, `RestartHandle`, `ShutdownHandle`, `RuntimeEnvironmentBroadcaster` — referenced from backends, SNI, DBus server, supervisor, main wiring.
- `ControlCommand`, `effective_dbus_name`, `DBUS_BASE_NAME`/`DAEMON_BUS_NAME_PREFIX`/`DBUS_PATH`/`DBUS_INTERFACE` — referenced from CLI control dispatch, DBus server, SNI Dbus mode, persistent DBus reconnector, GNOME focus subscription path.
- `DynError` alias — used pervasively under the lifecycle/supervisor cluster.

These belong in shallow modules (`config.rs`, `focus.rs`, `kanata.rs`, `env.rs`, `broadcasters.rs`, `dbus_naming.rs`, `constants.rs`) that everyone else can `use crate::env::Environment;` against.

---

## 3. Target module tree

```
src/daemon/
  main.rs                     # CLI bootstrap, run_once, signal wiring, mod declarations, top-level orchestration. Re-exports for test access.
  constants.rs                # All shared string constants: DBUS_*, GNOME_FOCUS_*, KDE_*, LOGIND_*, KDE_KWIN_*, KDE_RUNTIME_QUERY_MODE_*.
  errors.rs                   # `pub(crate) type DynError = Box<dyn std::error::Error + Send + Sync>;`. Also any free-standing error enums that escape one module (`DbusSuffixError`, `LogindSessionPathResolutionError`).
  env.rs                      # Environment, RunOutcome, SessionKind, DesktopFlavor, BackendKind, RuntimeTarget, DesktopCapabilities, LifecycleSnapshot, session_type_to_session_kind, session_type_indicates_native_terminal, resolve_desktop_flavor, resolve_runtime_target, runtime_target_from_wayland_startup_session_type_hint, target_requires_session_bus, startup_environment_to_snapshot, detect_environment.
  args.rs                     # Args (clap derive), parse_dbus_suffix_arg, resolve_install_gnome_extension, resolve_control_command, TrayFocusOnly.
  dbus_naming.rs              # DbusSuffixError, sanitize_dbus_suffix, derive_default_dbus_suffix, resolve_dbus_suffix, effective_dbus_name, is_daemon_bus_name.
  autostart.rs                # resolve_binary_path, autostart_dir, autostart_desktop_path, escape_desktop_exec_arg, build_autostart_desktop_content, autostart_passthrough_args, install_autostart_desktop, uninstall_autostart_desktop.
  config.rs                   # Rule, NativeTerminalRule, ConfigEntry, Config, WindowInfo, load_config, match_pattern.
  focus.rs                    # FocusAction, FocusActions, FocusHandler (and its big impl block).
  broadcasters.rs             # StatusSnapshot, LayerSource, StatusBroadcaster, RestartHandle, PauseBroadcaster, RuntimeEnvironmentBroadcaster, ShutdownHandle, wait_for_restart_or_shutdown.
  kanata.rs                   # Kanata wire types (Change/LayerChange/RequestLayerNames/ActOnFakeKey/Layer/RequestFakeKey/FakeKey *Msg + *Payload), KanataClientInner, pub struct KanataClient, ShutdownGuard.
  pause.rs                    # pause_daemon, unpause_daemon, UnpauseContext, local_sni_unpause_context, plus the test-only record/take_unpause_request_environment_for_test slot.
  focus_pipeline.rs           # execute_focus_actions, extract_focus_layer, update_status_for_focus, handle_focus_event, native_terminal_window, resolve_sni_focus_only.
  control/
    mod.rs                    # ControlCommand, ControlDispatch.
    client.rs                 # send_control_command, send_control_command_with_connection, BroadcastEntryReport, BroadcastReport, enumerate_daemon_names, send_control_command_broadcast.
    server.rs                 # DbusWindowFocusService + its zbus `#[interface]` impl + DbusServiceRegistration + register_dbus_service[_with_runtime_environment] + environment_requires_focus_query_connection + resolve_runtime_unpause_context.
    persistent.rs             # dbus_reconnect_delay, wait_for_dbus_reconnect_retry, PersistentDbusServiceGuard, start_persistent_dbus_service[_with_connector], run_persistent_dbus_service_with_connector.
  lifecycle/
    mod.rs                    # LifecycleProvider enum + impl (keeps build/build_with_logind_factory/is_continuous/next_snapshot). Re-exports below.
    snapshot.rs               # snapshot_no_session helper (and any decode helpers that don’t fit logind.rs cleanly).
    startup.rs                # StartupSnapshotProvider + impl.
    logind.rs                 # LogindLifecycleProvider, verify_logind_lifecycle_monitor_prerequisites, validate_active_logind_session_type, fail_fast_lifecycle_monitor, expect_some_or_fail_fast, expect_or_fail_fast, monitor_logind_lifecycle, open_logind_session_monitor, decode_logind_lifecycle_snapshot_change, resolve_logind_session_path, is_logind_no_session_error, LogindSessionPathResolutionError, is_logind_empty_object_path, parse_logind_object_path[_from_structure], decode_logind_object_path_reply, logind_object_path_from_value, resolve_logind_display_session_path, LogindDisplayPathChange, decode_logind_display_path_change, LogindDisplayChangeAction, apply_logind_display_change, wait_for_logind_display_session_path.
  display_override.rs         # Shared display-override helpers used by both supervisor and backends: display_override_expected_session_type, is_valid_wayland_display_override, normalize_display_override, resolve_display_override_from_logind, display_override_backend_kind_for_environment, resolve_display_override_for_backend_kind, resolve_display_override_for_environment, plus test-only TestFocusQueryDisplayOverrideGuard, display_override_test_slot, set/resolve_test_focus_query_display_override (cfg-test + cfg-not(test) variants), TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE (line 3560), TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE (line 3563).
  supervisor/
    mod.rs                    # BackendContext, BackendHandle, SupervisorState, run_lifecycle_supervisor (+ _with_starter, _with_starter_and_resolver), wait_for_wayland_capability_recheck, wait_for_backend_completion_signal, poll_finished_backend_outcome, transition_runtime_target[_with_starter], stop_current_backend, start_backend, ensure_runtime_gnome_extension_setup, runtime_target_* helpers. (`BackendExit` and `map_run_outcome_to_backend_exit` live in `backends/mod.rs` — see §4.1 and PR-12.) Contains `pub(crate) use capabilities::*;`.
    capabilities.rs           # detect_desktop_capabilities, resolve_runtime_target_for_snapshot.
  backends/
    mod.rs                    # `pub(crate) trait FocusBackend` (see §4). Also: `RawFdWatcher`, `query_focus_for_env`, `apply_focus_for_env` (dispatch helpers shared by backends). `BackendExit`, `map_run_outcome_to_backend_exit` (trait return type and its mapping helper; keep colocated with the trait — see §4.1 and PR-12).
    gnome.rs                  # async fn run_gnome, GnomeFocusSignalSubscription, subscribe_to_gnome_focus_signal, query_gnome_focus + run_gnome_backend_task (the supervisor adapter for gnome).
    kde/
      mod.rs                  # async fn run_kde, KwinScriptGuard, build_kde_focus_push_script + run_kde_backend_task.
      script.rs               # build_kde_query_script, kwin_query_script_path, kwin_query_probe_script_path, kwin_runtime_script_path, KdeFocusQueryService, kwin_script_object_path, load_kwin_script.
      probe.rs                # resolve_kde_runtime_query_mode_with_retry, ensure_kde_scripting_ready, resolve_kde_runtime_query_mode, kwin_object_path_exists, unload_kwin_script_by_path, remove_kwin_probe_script_file, query_kde_focus.
    wayland/
      mod.rs                  # ToplevelWindow, WaylandState (struct + inherent impl + Default), async fn run_wayland, resolve_wayland_socket_path, connect_wayland_with_display_override, query_wayland_active_window, wayland_query_count (AtomicUsize counter), run_wayland_backend_task.
      protocols.rs            # `mod cosmic_workspace { ... }` and `mod cosmic_toplevel { ... }` (the wayland_scanner generators — same paths to XML, see §6).
      dispatch_wlr.rs         # Dispatch<ZwlrForeignToplevelManagerV1, ()> for WaylandState, Dispatch<ZwlrForeignToplevelHandleV1, ()> for WaylandState.
      dispatch_cosmic.rs      # Dispatch<ZcosmicToplevelInfoV1, ()>, Dispatch<ZcosmicToplevelHandleV1, ()>, Dispatch<ZcosmicWorkspaceManagerV1, ()>, Dispatch<ZcosmicWorkspaceGroupHandleV1, ()>, Dispatch<ZcosmicWorkspaceHandleV1, ()> all for WaylandState.
      dispatch_common.rs      # Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState, Dispatch<wl_output::WlOutput, ()> for WaylandState.
    x11.rs                    # X11State + impl, async fn run_x11, query_x11_active_window + run_x11_backend_task.
    linux_console.rs          # run_linux_console_backend_task (fold body inline; ~20 LOC).
  sni/
    mod.rs                    # SniControl, SniControlMode, SniRuntimeTransitionPlan, sni_control_mode_for_environment, plan_sni_runtime_transition, SniRuntimeWakeReason, wait_for_sni_runtime_wake_with_delay. Re-exports below.
    settings.rs               # DconfBackend (trait), ShellDconfBackend, SniSettingsStore, dconf_get_bool, dconf_set_bool, is_dconf_unavailable.
    state.rs                  # MenuRefresh, SniIndicatorState.
    indicator.rs              # SniIndicator (with `impl Tray`), start_sni_indicator, SniIndicatorRuntimeHandle, ACTIVE_SNI_WATCHER_TASKS + SniWatcherTaskGuard + sni_watcher_task_count (test-only).
    control_local.rs          # SniLocalControl (and any `impl` methods specific to local mode if extracted).
    control_dbus.rs           # SniDbusControl.
    control_ops.rs            # `trait SniControlOps`, `impl SniControlOps for SniControl`.
    guard.rs                  # SniGuard (with `disabled`, `runtime_managed`, `runtime_managed_with_builder`), build_sni_control_for_mode, SNI_RUNTIME_RETRY_INTERVAL.
  gnome_ext/
    mod.rs                    # setup_gnome_extension, print_gnome_extension_status, print_gnome_extension_install_instructions, ensure_gnome_extension.
    detection.rs              # GnomeDetectionMethod, GnomeExtensionStatus, gnome_state_name, parse_gnome_extension_state, GnomeDbusProbeResult, is_dbus_service_unavailable, gnome_extension_dbus_probe, gnome_extension_dbus_probe_with_connection, gnome_extension_status, wait_for_session_bus_name_owner, session_bus_name_has_owner.
    install.rs                # GNOME_EXTENSION_SRC_PATH, GNOME_EXTENSION_SCHEMA_FILE, GNOME_EXTENSION_SCHEMA_COMPILED, get_gnome_extension_fs_path, gnome_extension_fs_exists, pack_and_install_from_dir, install_gnome_extension, enable_gnome_extension. (GNOME_EXTENSION_UUID stays in constants.rs — moved there in PR-01.)
    embed.rs                  # `gnome_ext_file!` macro + EMBEDDED_* `include_str!` consts + compile_gnome_schemas + write_embedded_extension_to_dir. Feature-gated `#[cfg(feature = "embed-gnome-extension")]`. **See §6 for the relative-path concern.**
  tests.rs                    # unchanged; remains `#[cfg(test)] mod tests;` attached to main.rs (still reads via `use super::*` — see §6).
  integration_tests.rs        # unchanged.
```

`src/daemon/main.rs` after the refactor contains, in order: top of file `use` block for the few items needed by `main`/`run_once`; `mod constants; mod errors; mod env; mod args; mod dbus_naming; mod autostart; mod config; mod focus; mod focus_pipeline; mod broadcasters; mod kanata; mod pause; mod control; mod lifecycle; mod display_override; mod supervisor; mod backends; mod sni; mod gnome_ext;`; the `#[tokio::main] async fn main` and `async fn run_once` bodies (largely identical to the current ones, just calling into modules); `#[cfg(test)] pub(crate) use ...` re-exports for test access (see §6); and finally `#[cfg(test)] mod tests; #[cfg(test)] mod integration_tests;`.

---

## 4. Trait seams

### 4.1 `FocusBackend` — **new**
The four `async fn run_{gnome,kde,wayland,x11}` functions and the linux-console handler all share the supervisor contract: take a `BackendContext`-style bundle plus a shutdown signal, return a `Result<BackendExit, DynError>` (or `RunOutcome`, which already maps to `BackendExit`). Currently each is dispatched from a separate `run_*_backend_task` adapter inside the supervisor. A trait makes the dispatch table explicit and the supervisor’s `start_backend` switch arm into a generic call.

```rust
// src/daemon/backends/mod.rs
use std::future::Future;
use std::pin::Pin;

pub(crate) trait FocusBackend: Send + 'static {
    fn kind(&self) -> BackendKind;
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>>;
}
```

Where `BackendRunContext` is just the existing argument tuple (`KanataClient`, `Arc<Mutex<FocusHandler>>`, `StatusBroadcaster`, `PauseBroadcaster`, `RestartHandle`, `ShutdownHandle`, `effective_dbus_name`, plus the per-backend optional display override). One concrete impl per file: `GnomeBackend`, `KdeBackend`, `WaylandBackend`, `X11Backend`, `LinuxConsoleBackend`. **Earns its keep**: it removes the five-arm `match RuntimeTarget` inside `start_backend` and makes the “add another DE” story (hypothetical) one-file. It also gives integration tests a clean place to inject a stub backend without going through the supervisor’s closure-injection variant `run_lifecycle_supervisor_with_starter` (which exists today because there is no seam — the test seam is currently a closure parameter; the trait formalizes that).

**Trait shape — `Pin<Box<dyn Future>>`, not RPIT-in-traits (PR-00 finding, 2026-05-11)**: The project pins `rust-bin.stable.latest.default` in `flake.nix`; the installed toolchain is rustc 1.92.0. The supervisor's `start_backend` dispatches via `Box<dyn FocusBackend>`, so `FocusBackend` **must be dyn-compatible**. RPIT-in-traits (`fn run(...) -> impl Future<...> + Send`) is NOT dyn-compatible on rustc 1.92.0 — PR-00's probe confirmed E0038 ("the trait `FocusBackend` is not dyn compatible … because method `run` references an `impl Trait` type in its return type"). The trait therefore uses the boxed-future shape: `fn run(self: Box<Self>, ctx: BackendRunContext) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>>`. Each `impl` body wraps its async block in `Box::pin(async move { ... })`. This costs one allocation per backend start (the daemon starts a backend on session transitions — sub-second-frequency events), which is negligible. Do NOT use `#[async_trait]` — that would add the `async-trait` crate as a new dependency, which is banned per §1 non-goals; the boxed-future shape achieves the same dyn-compatibility manually.

**Impl-side skeleton**: Every backend impl uses this form:

```rust
impl FocusBackend for GnomeBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move { /* unpack ctx and call run_gnome(...).await */ })
    }
}
```

PR-00 confirmed: trait declaration compiles, dyn-dispatch via `Box<dyn FocusBackend>` compiles, and `tokio::spawn(b.run(ctx))` compiles (Send propagation verified).

**`RunOutcome` vs `BackendExit`**: `RunOutcome` is the per-backend exit reason returned by the individual `run_gnome`/`run_kde`/etc. free functions. It is mapped to `BackendExit` by `map_run_outcome_to_backend_exit`, which lives in `backends/mod.rs` alongside `BackendExit` and the `FocusBackend` trait (moved there in PR-12 step 0). The trait method returns `Result<BackendExit, DynError>`, which is consistent with the current semantics — each `XxxBackend::run` impl calls the free function and maps its result via `map_run_outcome_to_backend_exit`.

### 4.2 `DconfBackend` — **already exists, formalize the consumer**
Lives at lines 1466–1481 today, used only inside `SniSettingsStore`. `ShellDconfBackend` is the production impl; tests construct a stub via `SniSettingsStore::with_backend(Box<dyn DconfBackend>)`. Just move it under `sni/settings.rs`; nothing changes about the trait itself. **Earns its keep**: existing test seam (the `#[cfg(test)] fn with_backend` constructor in `SniSettingsStore`).

### 4.3 `SniControlOps` — **already exists, formalize the consumer**
Lives at lines 1740–1844. `SniIndicator` already holds an `Arc<dyn SniControlOps>`. Move to `sni/control_ops.rs`. The `impl SniControlOps for SniControl` (which dispatches to `SniLocalControl` / `SniDbusControl`) stays here. **Earns its keep**: `SniIndicator` already consumes the trait object (line 1848). Tests for SNI menu actions can drop in a stub if needed.

### 4.4 `LifecycleProvider` — **deliberate non-trait**
Currently an enum with exactly two variants (`Logind(LogindLifecycleProvider)`, `Startup(StartupSnapshotProvider)`), dispatching `next_snapshot` and `is_continuous`. Reasons to keep it as an enum (rejecting a trait conversion):
- Only two implementations, both shipped (no third on the horizon).
- The two impls differ in semantic mode (`is_continuous()` returns `true` only for the logind variant), and supervisor code already pattern-matches on `LifecycleProvider::Logind(_)` for capability rechecks. A trait would either expose `is_continuous` (preserving the smell) or force the supervisor to downcast (worse).
- No test double exists or is required — `LogindLifecycleProvider::new` is already replaceable via `build_with_logind_factory` (the existing closure seam), which is sufficient for tests.

Rationale matches the global CLAUDE.md “no abstractions for single-use code” and the task brief’s “reject candidates that only have one implementation and no test double.” Call this out in the PR description.

### 4.5 Other rejected candidates
- **`KanataClient`** — single implementation, internal TCP/queue state. No test double; tests use a real mock kanata TCP server on a free port. Adding a trait would force generics through the entire pipeline (every backend, SNI, DBus server). Not worth it.
- **`StatusBroadcaster`/`PauseBroadcaster`/`RestartHandle`/`ShutdownHandle`** — these are already minimal channel wrappers. Tests use them directly. Trait-ification would yield no test seam and pure noise.

---

## 5. PR breakdown

15 PRs. Each is independently revertable. Verification baseline `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check && nix build && nix run .#test` — abbreviated below as **V0**.

### PR-00: pre-flight — toolchain sanity, no-op refactor scaffold

**Executed 2026-05-11.** Findings recorded below; subsequent PRs assume these outcomes.

- **Scope**: No code moves. Two mandatory smoke probes whose outcomes decide downstream PR-09 / PR-12 details.

  1. **`wayland_scanner` macro path resolution from a nested module** — inside a temporary `mod _pr00_probe;` placed under `src/daemon/`, compile both `wayland_scanner::generate_interfaces!("src/protocols/cosmic-workspace-unstable-v1.xml")` AND `wayland_scanner::generate_client_code!("src/protocols/cosmic-workspace-unstable-v1.xml")` inside a deeply-nested `pub mod probe_cosmic_workspace { pub mod __interfaces { ... } ... }`. **Result (rustc 1.92.0, wayland-scanner 0.31.8): COMPILES.** The macros resolve XML paths relative to `CARGO_MANIFEST_DIR`. PR-09 proceeds with the original path strings unchanged.

  2. **`FocusBackend` trait shape** — compile a probe trait `pub trait FocusBackend: Send + 'static { fn run(self: Box<Self>, ctx: BackendRunContext) -> RETURN; }` with two candidate `RETURN` types and verify three properties: (a) the trait declaration compiles; (b) `Box<dyn FocusBackend>` dispatch compiles (dyn-compatibility); (c) `tokio::spawn(b.run(ctx))` compiles (Send propagation).
     - **Candidate 1: RPIT-in-traits, `impl Future<Output = ...> + Send` — REJECTED.** Trait declares fine, but `Box<dyn FocusBackend>` fails with E0038 ("not dyn compatible … because method `run` references an `impl Trait` type in its return type"). RPIT-in-traits is not dyn-compatible on rustc 1.92.0.
     - **Candidate 2: boxed-future, `Pin<Box<dyn Future<Output = ...> + Send + 'static>>` — ACCEPTED.** All three checks pass. The trait is dyn-compatible, dispatch via `Box<dyn FocusBackend>` works, and the returned future spawn-s cleanly.
     - **Decision**: §4.1 and PR-12 step 2 use the `Pin<Box<dyn Future ...>>` shape. Each impl uses `Box::pin(async move { ... })`.

- **Import changes**: none (probe module added and removed atomically in this PR).
- **Verification**: `cargo check --bin kanata-switcher` (both probe variants); full V0 with the probe removed.
- **Risk**: zero — purely additive temporary code. Rollback: delete probe module, revert main.rs `mod _pr00_probe;` line.

### PR-01: extract `constants.rs`, `errors.rs`, `env.rs`
- **Scope, moves to `constants.rs`**: every `const` from lines 91–125 plus `GNOME_EXTENSION_SRC_PATH`, `GNOME_EXTENSION_SCHEMA_FILE`, `GNOME_EXTENSION_SCHEMA_COMPILED` from the GNOME ext block (these constants are referenced from both install and embed paths). Move `DCONF_FOCUS_ONLY_KEY` here too.

  **Complete `const` inventory and module assignments** (constants defined outside the 91–125 opening block):
  - `AUTOSTART_DESKTOP_FILENAME` (line 346), `AUTOSTART_PASSTHROUGH_OPTIONS` (347), `AUTOSTART_ONESHOT_OPTIONS` (359) → `autostart.rs` (used only by autostart code).
  - `BROADCAST_PER_CALL_TIMEOUT` (line 648) → `control/client.rs` (used only by broadcast dispatch).
  - `NATIVE_TERMINAL_RULE_INDEX` (line 946) → `config.rs` (used only by config/matching).
  - `SNI_*` constants (lines 1453–1464, eight constants) → `sni/` (used only by SNI code).
  - `WAYLAND_CAPABILITY_RECHECK_INTERVAL` (line 3345) → `supervisor/mod.rs` (used only by the lifecycle supervisor).
  - `GNOME_SHELL_BUS_NAME` (line 6187), `GNOME_SHELL_OBJECT_PATH` (6188), `GNOME_SHELL_EXTENSIONS_INTERFACE` (6189), `DBUS_ERROR_SERVICE_UNKNOWN` (6190), `DBUS_ERROR_NAME_HAS_NO_OWNER` (6191), `DBUS_ERROR_UNKNOWN_METHOD` (6192) → `constants.rs`. Rationale: `GNOME_SHELL_BUS_NAME` is used by `detect_desktop_capabilities` in `supervisor/capabilities.rs`, making it cross-cluster; the three `DBUS_ERROR_*` constants are generic DBus error strings also usable cross-cluster.
  - `DBUS_RECONNECT_DELAYS_MS` (line 7563) → `control/persistent.rs` (used only by the persistent DBus reconnector).
  - `SNI_RUNTIME_RETRY_INTERVAL` (line 1708) → `sni/guard.rs` (used only by SNI guard — see §8 item 7).

  Spell each of these assignments out in the PR scope list so the executor does not miss them.
- **Moves to `errors.rs`**: `type DynError = Box<dyn std::error::Error + Send + Sync>;` (line 2864).
- **Moves to `env.rs`**: `Environment`, `RunOutcome`, `SessionKind`, `DesktopFlavor`, `BackendKind`, `RuntimeTarget`, `DesktopCapabilities`, `LifecycleSnapshot`, `session_type_to_session_kind`, `session_type_indicates_native_terminal`, `resolve_desktop_flavor`, `resolve_runtime_target`, `runtime_target_from_wayland_startup_session_type_hint`, `target_requires_session_bus`, `startup_environment_to_snapshot`, `impl Environment` (lines 4887–5086), `detect_environment` (lines 5053–5086).
- **LOC moved**: ~250.
- **Visibility**: every item becomes `pub(crate)`. `pub enum Environment` and `pub struct KanataClient` need to **stay `pub`** because they ride on the `[[bin]]` boundary (no external consumer exists for a bin crate, but the existing `pub` is preserved for consistency).
- **Import changes**: `main.rs` adds `mod constants; mod errors; mod env;` and `use constants::*; use errors::DynError; use env::*;` (or specific item lists). Test files: `use super::*;` continues to expose these because `main.rs` will `pub(crate) use {constants::*, errors::*, env::*};` at the top of `main.rs`. **Always re-export from `main.rs`** for `tests.rs`/`integration_tests.rs` — they `use super::*` against `main.rs`, not against the leaf modules.
- **Verification**: V0.
- **Risk**: visibility leaks. Rollback: revert single commit.

### PR-02: extract `errors`-adjacent enums and `dbus_naming.rs`
- **Scope, moves to `errors.rs`**: `DbusSuffixError` + `impl Display + Error` (lines 127–154) — only because it’s a free-floating error type referenced from `args` and `dbus_naming`.
- **Moves to `dbus_naming.rs`**: `sanitize_dbus_suffix`, `derive_default_dbus_suffix`, `resolve_dbus_suffix`, `effective_dbus_name`, `is_daemon_bus_name` (lines 156–230).
- **LOC moved**: ~110.
- **Visibility**: all `pub(crate)`.
- **Import changes**: `main.rs` adds `mod dbus_naming; use dbus_naming::*;` and re-exports for tests.
- **Verification**: V0.
- **Risk**: low — pure helpers with no statics. Rollback: revert.

### PR-03: extract `config.rs` and `focus.rs`
- **Scope, moves to `config.rs`**: `Rule`, `NativeTerminalRule`, `ConfigEntry`, `Config`, `WindowInfo`, `load_config`, `match_pattern` (lines 697–917).
- **Moves to `focus.rs`**: `FocusAction`, `FocusActions`, `impl FocusActions`, `FocusHandler` and its full impl block (lines 919–1244).
- **LOC moved**: ~530.
- **Visibility**: `pub(crate)`. The `Rule`, `WindowInfo`, `FocusActions`, `FocusAction`, `FocusHandler` are all heavily used in `tests.rs` — re-export from `main.rs` (`pub(crate) use crate::{config::*, focus::*};`).
- **Import changes**: `main.rs` adds `mod config; mod focus;`. Inside the new files: `use crate::env::Environment;` is unnecessary here because focus only uses `WindowInfo` (no env). Note: `WindowInfo` has a `is_native_terminal: bool` field used by `tests.rs` (see `fn win(...)` in tests.rs at line 24).
- **Verification**: V0. `cargo test` will exercise `FocusHandler` heavily — green test run is the strongest signal this PR is correct.
- **Risk**: medium — `FocusHandler::handle` is the daemon’s heart, touched by most tests. Behaviour-preserving move only. Rollback: revert.

### PR-04: extract `args.rs`, `autostart.rs`, `broadcasters.rs`, `kanata.rs`
- **Scope, moves to `args.rs`**: `TrayFocusOnly` (lines 259–278), `Args`, `parse_dbus_suffix_arg`, `resolve_install_gnome_extension`, `resolve_control_command` (lines 280–402).
- **Moves to `autostart.rs`**: lines 403–559 (all 8 fns).
- **Moves to `broadcasters.rs`**: `StatusSnapshot`, `LayerSource`, `StatusBroadcaster`, `RestartHandle`, `PauseBroadcaster`, `RuntimeEnvironmentBroadcaster`, `ShutdownHandle`, `wait_for_restart_or_shutdown` (lines 1246–1464).
- **Moves to `kanata.rs`**: all kanata protocol message structs (lines 4246–4317), `KanataClientInner`, `pub struct KanataClient`, `impl KanataClient`, `ShutdownGuard` (lines 4319–4885).
- **LOC moved**: ~1140. Per-destination estimates: `args.rs` ≈ 122 LOC, `autostart.rs` ≈ 156 LOC, `broadcasters.rs` ≈ 219 LOC, `kanata.rs` ≈ 640 LOC — all well under the §1 budget of 1200 LOC per file.
- **Visibility**: `pub(crate)` everywhere except `pub struct KanataClient` and `pub enum Environment` (already pub, preserved). `Args` needs to be `pub(crate)` since `tests.rs` uses `clap::Parser` to construct one (line 2 of tests.rs imports `clap::Parser`).
- **Import changes**: each new file `use crate::{constants::*, errors::DynError, env::*, broadcasters::*, focus::FocusHandler};` as needed. `main.rs` adds four `mod` declarations and re-exports for tests.
- **Verification**: V0. Pay particular attention to `nix run .#test` — `kanata.rs` is touched by every integration test that connects to a mock kanata TCP server.
- **Risk**: medium-high — `KanataClient` is held by reference from every backend file we haven’t moved yet. They keep their existing call sites; we’re just relocating the definition. Rollback: revert. **Consider splitting** into 4a (args+autostart), 4b (broadcasters), 4c (kanata+ShutdownGuard) for reviewability — each sub-PR is independently understandable and reviewable.

### PR-05: extract `control/` (client + types)
- **Scope, moves to `control/mod.rs`**: `ControlCommand` + `impl ControlCommand` (lines 232–256), `ControlDispatch` (lines 561–564).
- **Moves to `control/client.rs`**: `send_control_command`, `send_control_command_with_connection`, `BroadcastEntryReport`, `BroadcastReport`, `enumerate_daemon_names`, `send_control_command_broadcast` (lines 566–696).
- **LOC moved**: ~170.
- **Visibility**: `pub(crate)`.
- **Import changes**: `main.rs` adds `mod control; use control::{ControlCommand, ControlDispatch}; use control::client::*;`. `run_once` continues to call `send_control_command` and `effective_dbus_name` unchanged.
- **Verification**: V0.
- **Risk**: low — pure functions with no shared state. Rollback: revert.

### PR-06: extract `pause.rs` and `focus_pipeline.rs`
- **Scope, moves to `pause.rs`**: `UnpauseContext` (lines 1613–1617), `local_sni_unpause_context` (lines 1669–1686), `pause_daemon` (4147), `unpause_daemon` (4184), and the test-only `record_unpause_request_environment_for_test` + `take_unpause_request_environment_for_test` + their static slot `TEST_LAST_UNPAUSE_REQUEST_ENV` (line 4227).
- **Moves to `focus_pipeline.rs`**: `execute_focus_actions`, `extract_focus_layer`, `update_status_for_focus`, `handle_focus_event`, `native_terminal_window`, `resolve_sni_focus_only` (lines 2119–2213).
- **LOC moved**: ~210.
- **Visibility**: `pub(crate)`.
- **Visibility widening sub-step (D19)**: Before extracting `pause.rs`, widen `apply_focus_for_env` and `query_focus_for_env` to `pub(crate)` in `main.rs`. `unpause_daemon` (moving to `pause.rs`) calls `apply_focus_for_env` (main.rs line 2546), which remains private until PR-11. Without this widening the new `pause.rs` fails to compile with "function `apply_focus_for_env` is private". Revert both widenings in PR-11 when `apply_focus_for_env`/`query_focus_for_env` move to `backends/mod.rs`.
- **Import changes**: both files `use crate::{kanata::KanataClient, focus::*, broadcasters::*, config::WindowInfo, env::Environment, pause::UnpauseContext};`. **Important**: `UnpauseContext.connection` is a `zbus::Connection` — keep the type re-exported via `pub(crate) use zbus::Connection;` from the module or just import directly in the file. **Cross-reference fix (D21)**: After moving `UnpauseContext` to `pause.rs`, the remaining call sites in `main.rs` (SNI `SniLocalControl` field at line 1628; DBus server `resolve_runtime_unpause_context` at 6871, `Unpause` method handler at 6873/6900) still reference `UnpauseContext` by name. In this same PR either add `pub(crate) use pause::UnpauseContext;` as a re-export in `main.rs` or update each remaining call site with an explicit `use crate::pause::UnpauseContext;`. These consumers move to their own modules in PR-13/PR-14; the re-export (or per-site import) can then be deleted. Apply this same pattern to every PR that extracts a type whose consumers still reside in `main.rs`.
- **Verification**: V0.
- **Risk**: low. Rollback: revert.

### PR-07: extract `lifecycle/`
- **Scope, moves to `lifecycle/mod.rs`**: `LifecycleProvider` enum + `impl LifecycleProvider` (lines 3203–3243).
- **Moves to `lifecycle/startup.rs`**: `StartupSnapshotProvider` + impl (lines 2867–2882).
- **Moves to `lifecycle/logind.rs`**: everything from lines 2571–3201 *plus* logind-specific items in 2867–3201: all the parsing/decoding/resolution helpers, `LogindLifecycleProvider` + impl, `verify_logind_lifecycle_monitor_prerequisites`, `validate_active_logind_session_type`, `fail_fast_lifecycle_monitor`, `expect_some_or_fail_fast`, `expect_or_fail_fast`, `monitor_logind_lifecycle`, `open_logind_session_monitor`, `decode_logind_lifecycle_snapshot_change`, `snapshot_no_session`.
- **LOC moved**: ~640.
- **Visibility**: `pub(crate)` for the enum and the providers; helpers stay `pub(super)` within the module.
- **Import changes**: `main.rs` adds `mod lifecycle;` and re-exports `LifecycleProvider` for tests. Inside `logind.rs`: `use crate::{constants::*, errors::DynError, env::*};`.
- **Verification**: V0. `nix run .#test` exercises logind paths via mock zbus where available; rest are non-tested.
- **Risk**: medium — large move, many helper functions, but they’re internally cohesive (all logind concerns). The `wait_for_logind_display_session_path` and `monitor_logind_lifecycle` are the hairiest. Rollback: revert single commit.

### PR-08: extract `supervisor/` and `display_override.rs`
- **Scope, moves to `supervisor/capabilities.rs`**: `detect_desktop_capabilities` (3347), `resolve_runtime_target_for_snapshot` (3358).
- **Moves to top-level `src/daemon/display_override.rs`** (NOT under `supervisor/`): `display_override_expected_session_type`, `is_valid_wayland_display_override`, `normalize_display_override`, `resolve_display_override_from_logind`, `display_override_backend_kind_for_environment`, `TestFocusQueryDisplayOverrideGuard`, `display_override_test_slot`, both `set_test_focus_query_display_override` and `resolve_test_focus_query_display_override` variants (cfg-test and cfg-not-test), `resolve_display_override_for_backend_kind`, `resolve_display_override_for_environment` (lines 3479–3661), `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE` (line 3560), and `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` (line 3563). Rationale: `resolve_display_override_for_environment` is called from both `supervisor` (lines 3692/3706) and `query_focus_for_env` (lines 2530/2536, which moves to `backends/mod.rs` in PR-11). Placing it in `supervisor/` would create a backends→supervisor dependency cycle. A top-level `display_override.rs` (sibling of `supervisor/` and `backends/`) is the correct shared location.
- **Moves to `supervisor/mod.rs`**: everything else in 3203–4145 *except* the items already moved in PR-07, the display-override items above, and `BackendExit` and `map_run_outcome_to_backend_exit`, which stay in `main.rs` (widened to `pub(crate)`) until PR-12 moves them into `backends/mod.rs` together with the `FocusBackend` trait. This avoids creating `backends/mod.rs` ahead of PR-09. `BackendContext`, `BackendHandle` + impl, `runtime_target_label`/`runtime_target_is_wayland_family`/`runtime_target_to_environment`, the five `run_*_backend_task` stubs (these stay here because they live above the per-backend modules — they’re the dispatch layer; they will be deleted in PR-12 when `FocusBackend` lands and the supervisor calls the trait directly, but for now they keep the supervisor compiling), `ensure_runtime_gnome_extension_setup`, `start_backend`, `SupervisorState`, `transition_runtime_target[_with_starter]`, `stop_current_backend`, `run_lifecycle_supervisor[_with_starter][_with_starter_and_resolver]`, `wait_for_wayland_capability_recheck`, `wait_for_backend_completion_signal`, `poll_finished_backend_outcome`. `supervisor/mod.rs` must contain `pub(crate) use capabilities::*;` so that the outer `supervisor::*` glob in the re-export block exposes capabilities items to the test scope. `display_override` items do NOT need re-exporting through supervisor — they are imported directly as `crate::display_override::*`.
- **Visibility widening sub-step**: Before moving the supervisor body, change `run_gnome`, `run_kde`, `run_wayland`, `run_x11`, `BackendExit`, `map_run_outcome_to_backend_exit` in `main.rs` from private to `pub(crate)` (so `crate::run_gnome` and `crate::BackendExit` resolve from the new `supervisor/mod.rs`). Similarly widen to `pub(crate)` any `query_*_focus` / `query_*_active_window` functions that remain in `main.rs` when `query_focus_for_env` (also still in `main.rs`) calls them after PR-09/PR-10: specifically `query_gnome_focus`, `query_kde_focus`, `query_x11_active_window`, `query_wayland_active_window`. The `BackendExit`/`map_run_outcome_to_backend_exit` widenings are reverted in PR-12 when they move to `backends/mod.rs`. The `run_*`/`query_*` widenings are reverted when those functions move into their permanent modules in PR-09/PR-10/PR-11.
- **LOC moved**: ~930 (≈746 to `supervisor/`, ≈183 to `display_override.rs`); the 13 LOC of `BackendExit`/`map_run_outcome_to_backend_exit` stay in `main.rs` (widened to `pub(crate)`) until PR-12 moves them.
- **Visibility**: `pub(crate)`. The `_with_starter` and `_with_starter_and_resolver` variants are test seams — keep `pub(crate)`.
- **Import changes**: `mod display_override; mod supervisor;` in `main.rs`. Inside `supervisor/`: heavy `use` of `crate::{kanata::KanataClient, focus::*, broadcasters::*, env::*, errors::DynError, lifecycle::*, focus_pipeline::*, control::server::*, gnome_ext::setup_gnome_extension, display_override::*}`. **Circular dep watch**: `supervisor` imports `backends::*` (the `run_*_backend_task` adapters call into `run_gnome`/`run_kde`/`run_wayland`/`run_x11` which aren’t moved yet — they still live in `main.rs` at this point). Resolution: this PR leaves `run_*_backend_task` referencing items in `main.rs` via `crate::run_gnome`, etc. **PR-09–PR-11 are sequenced after this to satisfy that.**
- **Verification**: V0.
- **Risk**: high — supervisor is the most central module. Strongly consider splitting into 8a (capabilities + display_override) and 8b (the rest). Rollback: revert.

### PR-09: extract `backends/wayland/` (including the `wayland_scanner` modules)
- **Scope, moves to `backends/wayland/protocols.rs`**: `mod cosmic_workspace { ... }` and `mod cosmic_toplevel { ... }` (lines 51–89). **The `wayland_scanner::generate_interfaces!`/`generate_client_code!` paths are relative to the crate root, not to the source file** (they go through `proc_macro` and resolve via `CARGO_MANIFEST_DIR`). The current strings `"src/protocols/cosmic-workspace-unstable-v1.xml"` and `"src/protocols/cosmic-toplevel-info-unstable-v1.xml"` continue to resolve correctly from any source file. Verify in PR-00 by writing a one-off probe; if the macro turns out to be source-file-relative on this version, fall back to constructing the path explicitly with `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")`. The user’s `wayland-scanner` version is `=0.31.8`; this version uses `CARGO_MANIFEST_DIR` (the crate-relative path is correct).
- **Moves to `backends/mod.rs`**: `RawFdWatcher` (main.rs lines 2214–2228). It is used by both `run_wayland` (line 5364) AND `run_x11` (line 5619), so it belongs in the shared backends module, not in `wayland/`. `backends/x11.rs` imports it as `use crate::backends::RawFdWatcher;`.
- **Moves to `backends/wayland/mod.rs`**: `ToplevelWindow` (5088), `WaylandState` + inherent impl (5094), `WaylandProtocol` enum, `async fn run_wayland`. Also: `resolve_wayland_socket_path` (2230), `connect_wayland_with_display_override` (2248), `query_wayland_active_window` (2261), `wayland_query_count` + its `WAYLAND_QUERY_COUNTER` static (2294/2307), `run_wayland_backend_task` (3410, deleted/inlined when trait lands in PR-12).
- **Moves to `backends/wayland/dispatch_common.rs`**: `Dispatch<wl_registry::WlRegistry, GlobalListContents>` and `Dispatch<wl_output::WlOutput, ()>` impls.
- **Moves to `backends/wayland/dispatch_wlr.rs`**: `Dispatch<ZwlrForeignToplevelManagerV1, ()>` and `Dispatch<ZwlrForeignToplevelHandleV1, ()>` impls.
- **Moves to `backends/wayland/dispatch_cosmic.rs`**: the five cosmic dispatch impls.
- **LOC moved**: ~480.
- **Visibility**: `pub(crate)` for the `mod cosmic_*` modules so the dispatch impl files can `use crate::backends::wayland::protocols::cosmic_toplevel::...`.
- **Import changes**: `main.rs` removes the top-level `mod cosmic_workspace`/`mod cosmic_toplevel` and adds `mod backends;` (with `pub(crate) mod wayland;` inside `backends/mod.rs`). **Important**: the existing `use cosmic_toplevel::{...}; use cosmic_workspace::{...};` in `main.rs` (lines 81–89) is no longer needed in `main.rs`; that import moves into `backends/wayland/dispatch_cosmic.rs`. **Also**: inside `mod cosmic_toplevel` (main.rs line 72) the code reads `use crate::cosmic_workspace::__interfaces::*;` and (line 77) `use crate::cosmic_workspace::*;`. After the move, `crate::cosmic_workspace` no longer resolves. Update both paths to `super::cosmic_workspace::__interfaces::*` and `super::cosmic_workspace::*` respectively (relative within `protocols.rs`). Verify no other `crate::cosmic_*` or `crate::cosmic_toplevel::*` absolute paths appear in the moved bodies.
- **Submodule re-export (D24)**: `backends/wayland/mod.rs` must contain `pub(crate) use {protocols::*, dispatch_common::*, dispatch_wlr::*, dispatch_cosmic::*};` so that the outer `backends::wayland::*` glob in the §6 re-export block reaches submodule items.
- **Clippy audit**: After moving, run `cargo clippy --all-targets -- -D warnings` and audit any lint that fires inside `backends/wayland/dispatch_*.rs` on `use crate::backends::wayland::protocols::cosmic_*` lines. Suppress narrowly if needed; do NOT add `#![allow(clippy::all)]` to `dispatch_*.rs` — that hides real lints.
- **Verification**: V0. **Highest scanner-resolution risk PR.** If wayland_scanner errors on macro expansion, fall back to the explicit `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")` form before reverting.
- **Risk**: high (macro path resolution). Rollback: revert.

### PR-10: extract `backends/x11.rs` and `backends/linux_console.rs`
- **Scope, moves to `backends/x11.rs`**: `X11State` + impl (5460), `async fn run_x11` (5589), `query_x11_active_window` (2298), `run_x11_backend_task` (3427). Also move the `x11rb::atom_manager! { pub X11Atoms: X11AtomsCookie { ... } }` block at main.rs lines 5452–5458 into `backends/x11.rs` adjacent to `X11State`. Note: `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE` (line 3560) and `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` (line 3563) are kept in `crate::display_override` (moved there in PR-08 — they’re test plumbing, not a runtime X11 concern). X11 backend imports them via `use crate::display_override::*` — it does NOT own them.
- **Moves to `backends/linux_console.rs`**: `run_linux_console_backend_task` (3444, ~18 LOC).
- **LOC moved**: ~310.
- **Visibility**: `pub(crate)`.
- **Import changes**: `mod x11; mod linux_console;` inside `backends/mod.rs`. `backends/x11.rs` uses `crate::backends::RawFdWatcher` (from `backends/mod.rs`) — NOT `crate::backends::wayland::RawFdWatcher`.
- **Verification**: V0. `cargo test x11` runs Xvfb-backed tests at displays `:100/:101/:102`.
- **Risk**: low — X11 is self-contained except for the focus-pipeline call. Rollback: revert.

### PR-11: extract `backends/gnome.rs` and `backends/kde/`
- **Scope, moves to `backends/gnome.rs`**: `async fn run_gnome` (7225), `GnomeFocusSignalSubscription` (7268), `subscribe_to_gnome_focus_signal` (7278), `query_gnome_focus` (2495), `run_gnome_backend_task` (3377).
- **Moves to `backends/kde/script.rs`**: `kwin_query_script_path` (2309), `kwin_query_probe_script_path` (2318), `kwin_runtime_script_path` (2327), `KdeFocusQueryService` (2334), `kwin_script_object_path` (2354), `load_kwin_script` (2367), `build_kde_query_script` (2417), `build_kde_focus_push_script` (7422), and the static `KDE_QUERY_COUNTER` (line 2305).
- **Moves to `backends/kde/probe.rs`**: `resolve_kde_runtime_query_mode_with_retry` (6907), `ensure_kde_scripting_ready` (6947), `resolve_kde_runtime_query_mode` (6988), `kwin_object_path_exists` (7043), `unload_kwin_script_by_path` (7056), `remove_kwin_probe_script_file` (7072), `environment_requires_focus_query_connection` (7082), `query_kde_focus` (2444).
- **Moves to `backends/kde/mod.rs`**: `KwinScriptGuard` (7342), `async fn run_kde` (7445), `run_kde_backend_task` (3393). Also `query_focus_for_env` (2515) and `apply_focus_for_env` (2546) — these are dispatch helpers; place in `backends/mod.rs` (not kde/mod.rs).
- **Submodule re-export (D24)**: `backends/kde/mod.rs` must contain `pub(crate) use {script::*, probe::*};` so that `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, `build_kde_query_script`, `query_kde_focus`, and `ensure_kde_scripting_ready` are reachable through `backends::kde::*` in `tests.rs`.
- **LOC moved**: ~800.
- **Visibility**: `pub(crate)`.
- **Import changes**: `pub(crate) mod gnome; pub(crate) mod kde;` inside `backends/mod.rs`. `query_focus_for_env`/`apply_focus_for_env` live in `backends/mod.rs` since they dispatch across all backends. `backends/mod.rs` imports `use crate::display_override::*;` (NOT `crate::supervisor::display_override::*` — display_override is a top-level module per PR-08).
- **Verification**: V0.
- **Risk**: medium. KDE is the largest single-backend module. Rollback: revert.

### PR-12: introduce `FocusBackend` trait, retire the `run_*_backend_task` adapters
- **Scope**: add `trait FocusBackend` to `backends/mod.rs`. Implement it for each backend module’s primary struct: `GnomeBackend`, `KdeBackend`, `WaylandBackend`, `X11Backend`, `LinuxConsoleBackend`. Each struct holds the existing per-backend argument bundle. In the supervisor’s `start_backend`, replace the per-target `match` arms that call `run_*_backend_task` with a `Box<dyn FocusBackend>` construction + `.run(...).await`. Delete the five `run_*_backend_task` adapter functions.

  **Five-step procedure**:
  0. Move `BackendExit` and `map_run_outcome_to_backend_exit` (currently in `main.rs`, widened to `pub(crate)` in PR-08) into `backends/mod.rs`, and revert the temporary `pub(crate)` widening. These must live in `backends/` because `FocusBackend::run` returns `Result<BackendExit, DynError>` — placing them in `supervisor/` would force every backend impl to `use crate::supervisor::BackendExit`, creating a backends→supervisor edge that contradicts the acyclic dependency invariant (§7 risk 5). `supervisor/` imports `BackendExit` from `crate::backends::BackendExit`.
  1. Define `pub(crate) struct BackendRunContext { ... }` in `backends/mod.rs`, with fields matching the existing `BackendContext` plus `ShutdownHandle`.
  2. For each backend, add a small struct `XxxBackend` and `impl FocusBackend for XxxBackend` whose `run` body unpacks `BackendRunContext` and delegates to the existing free `async fn run_xxx`. **Use the `Pin<Box<dyn Future ...>>` shape** (PR-00 finding — RPIT-in-traits is dyn-incompatible on rustc 1.92.0): `fn run(self: Box<Self>, ctx: BackendRunContext) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> { Box::pin(async move { /* unpack ctx and call run_xxx(...).await */ }) }`. See §4.1 for rationale.
  3. In `supervisor::start_backend`, replace each `RuntimeTarget::Xxx => run_xxx_backend_task(...)` arm with `RuntimeTarget::Xxx => Box::new(XxxBackend::new(...)) as Box<dyn FocusBackend>`.
  4. Delete `run_gnome_backend_task`, `run_kde_backend_task`, `run_wayland_backend_task`, `run_x11_backend_task`, `run_linux_console_backend_task`.

- **LOC moved**: +213 / -150, net +63 (including the 13 LOC of `BackendExit`/`map_run_outcome_to_backend_exit` moved out of `main.rs`; plus `BackendRunContext` struct definition and per-backend `XxxBackend` boilerplate, partially offset by the deleted adapters).
- **Visibility**: `pub(crate) trait FocusBackend`.
- **Import changes**: supervisor `use crate::backends::FocusBackend;`. Each backend file gains a small `pub(crate) struct XxxBackend { /* fields = old args */ }; impl FocusBackend for XxxBackend { ... }` block. Existing free functions `run_gnome`/`run_kde`/`run_wayland`/`run_x11` stay as free fns called by the impl — that is the smaller diff.
- **Verification**: V0.
- **Risk**: medium — this is the first PR with semantic-shaped change (the dispatch path). Behaviour identical because the trait method body delegates to the same free function. Rollback: revert.

### PR-13: extract `control/server.rs` and `control/persistent.rs`
- **Scope, moves to `control/server.rs`**: `DbusWindowFocusService` + its zbus `#[interface]` impl (6759–6870), `resolve_runtime_unpause_context` (6871), `DbusServiceRegistration` + Drop (7089), `register_dbus_service` (7103), `register_dbus_service_with_runtime_environment` (7131).
- **Moves to `control/persistent.rs`**: `dbus_reconnect_delay` (7565), `wait_for_dbus_reconnect_retry` (7570), `PersistentDbusServiceGuard` + Drop (7587), `start_persistent_dbus_service` (7597), `start_persistent_dbus_service_with_connector` (7624), `run_persistent_dbus_service_with_connector` (7656).
- **LOC moved**: ~700.
- **Visibility**: `pub(crate)`.
- **Import changes**: `control/server.rs` heavily uses `crate::{kanata::*, focus::*, broadcasters::*, env::*, pause::*, backends::kde::probe::*}`. `control/persistent.rs` uses `crate::{control::server::*, broadcasters::*}`.
- **Verification**: V0.
- **Risk**: medium — the zbus `#[interface]` macro is sensitive to imports. Keep the `impl` block in the same file as the struct (we are). Rollback: revert.

### PR-14: extract `sni/`
- **Scope** (split across files per §3): all SNI items: trait, settings, state, indicator, control_local, control_dbus, control_ops, guard. Move `dconf_get_bool`/`dconf_set_bool`/`is_dconf_unavailable` into `sni/settings.rs` (they’re only used by SNI). Move `SNI_RUNTIME_RETRY_INTERVAL` (currently in main.rs near SNI guard) into `sni/guard.rs`.
- **LOC moved**: ~1100. **Recommend splitting** into 14a (`sni/settings.rs` + `sni/state.rs` + dconf helpers), 14b (`sni/control_{local,dbus,ops}.rs`), 14c (`sni/indicator.rs` + `sni/guard.rs` + transitions).
- **Visibility**: `pub(crate)` for everything used by `tests.rs` (the test file at lines 12–13 declares `SNI_WATCHER_TEST_LOCK` and uses `SniWatcherTaskGuard`, `ACTIVE_SNI_WATCHER_TASKS`, etc.). Re-export from `main.rs` for `use super::*` to keep working.
- **Import changes**: `mod sni;` in `main.rs`. SNI files import `crate::{kanata::KanataClient, focus::*, broadcasters::*, pause::*, control::client::send_control_command_with_connection, env::Environment, constants::*}`.
- **Verification**: V0. SNI tests are particularly numerous in `tests.rs`.
- **Risk**: medium-high (touch surface, but cohesive). Rollback: revert each sub-PR.

### PR-15: extract `gnome_ext/`
- **Scope, moves to `gnome_ext/detection.rs`**: `GnomeDetectionMethod` (6128), `GnomeExtensionStatus` (6135), `gnome_state_name` (6149), `parse_gnome_extension_state` (6163), `GnomeDbusProbeResult` (6194), `is_dbus_service_unavailable` (6200), `gnome_extension_dbus_probe[_with_connection]` (6220/6235), `gnome_extension_status` (6268), `wait_for_session_bus_name_owner` (6315), `session_bus_name_has_owner` (6407).
- **Moves to `gnome_ext/install.rs`**: `get_gnome_extension_fs_path`, `gnome_extension_fs_exists`, `pack_and_install_from_dir`, `install_gnome_extension`, `enable_gnome_extension`. `GNOME_EXTENSION_UUID` stays in `constants.rs` (already moved in PR-01).
- **Moves to `gnome_ext/embed.rs`** (all `#[cfg(feature = "embed-gnome-extension")]`): `gnome_ext_file!` macro, the 9 `EMBEDDED_*` consts, `compile_gnome_schemas`, `write_embedded_extension_to_dir`. **§6: the `include_str!` paths inside `gnome_ext_file!` use the literal string `"../../src/gnome-extension/..."` which is relative to the source file containing the `include_str!` invocation.** When the macro expansion moves from `src/daemon/main.rs` to `src/daemon/gnome_ext/embed.rs`, the relative path must change to `"../../../src/gnome-extension/..."` (three `..` instead of two). **The macro definition must be updated to `concat!("../../../", "src/gnome-extension", "/", $file)`** when moved.
- **Moves to `gnome_ext/mod.rs`**: `setup_gnome_extension` (6660), `ensure_gnome_extension` (6588), `print_gnome_extension_status` (6619), `print_gnome_extension_install_instructions` (6420).
- **LOC moved**: ~700.
- **Visibility**: `pub(crate)`.
- **Import changes**: `mod gnome_ext;` in `main.rs`. Inside `gnome_ext/`: `use crate::{constants::*, env::Environment};`.
- **Verification**: V0. **Specifically verify the `embed-gnome-extension` feature**: `cargo build --features embed-gnome-extension` and `cargo build --no-default-features`. The relative-path update is the single highest-risk item in this PR.
- **Risk**: high (relative-path resolution). Rollback: revert.

### Post-refactor sanity (no PR needed, but verify)
After PR-15, `main.rs` should be ~250–400 LOC. Confirm:
- No `struct`/`enum`/`trait`/`impl` block remains in `main.rs` apart from `#[cfg(test)] mod tests; #[cfg(test)] mod integration_tests;`.
- `main.rs` contains only: `use` block, module declarations, `pub(crate) use` re-exports for test access, `#[tokio::main] async fn main`, `async fn run_once`.
- `wc -l src/daemon/main.rs` reports ≤ 400.

---

## 6. Cross-cutting concerns

### Visibility policy
- Default: every moved item is `pub(crate)`.
- `pub(super)` is rarely useful here — modules are mostly siblings of `main.rs`, not deeply nested. Reserve it for helpers inside `lifecycle/logind.rs`, `backends/wayland/dispatch_*.rs`, and `backends/kde/probe.rs` that are only consumed by their sibling files.
- `pub` (no qualifier) is reserved for items that are already `pub` (`KanataClient`, `Environment`) — keep these as-is for consistency, even though there is no external consumer of a `[[bin]]` crate. Don’t downgrade.

### Re-export strategy from `main.rs`
The test files (`tests.rs`, `integration_tests.rs`) are attached as inner modules of `main.rs` and use `use super::*;`. This currently exposes every internal of `main.rs` to them. After extraction, internals live in sibling modules. To preserve `use super::*;` semantics without rewriting the test files (which the brief says to leave alone unless splitting them simplifies a PR — and splitting them doesn’t), `main.rs` should contain, immediately after the `mod` declarations:

```rust
#[cfg(test)]
pub(crate) use crate::{
    args::*,
    autostart::*,
    backends::{self, x11::*, gnome::*, kde::*, wayland::*, *},
    broadcasters::*,
    config::*,
    constants::*,
    control::{self, client::*, server::*, persistent::*, *},
    dbus_naming::*,
    display_override::*,
    env::*,
    errors::*,
    focus::*,
    focus_pipeline::*,
    gnome_ext::*,
    kanata::*,
    lifecycle::*,
    pause::*,
    sni::{self, settings::*, state::*, indicator::*, guard::*, control_local::*, control_dbus::*, control_ops::*, *},
    supervisor::{self, *},
};
```

This is verbose but mechanical, lives in one place, and is `#[cfg(test)]`-gated so it doesn’t pollute release builds. Alternative: emit each PR’s re-exports incrementally as items move. The verbose all-at-once block is preferable — it makes the test-access contract a single visible artefact, and PRs only adjust it when a name is added.

### Logging / `quiet*` flags
`quiet_focus` and `args.quiet` are read in `run_once` once and threaded into `KanataClient::new(...)` and `FocusHandler::new(..., quiet_focus)`. No module “owns” the global quiet state — it’s already plumbed through constructors. **No change needed** during the refactor: each constructor signature is preserved.

### Feature flag: `embed-gnome-extension`
See PR-15. The `gnome_ext_file!` macro must update its relative path from `"../../"` to `"../../../"` (because the source file moves one level deeper). Verify with `cargo build --features embed-gnome-extension` after PR-15.

### `wayland_scanner` macros and the `cosmic_workspace`/`cosmic_toplevel` modules
The macros `generate_interfaces!` and `generate_client_code!` take a path argument. **PR-00 mandates a smoke-test probe** (see PR-00 scope item 2) to determine whether the path is resolved relative to `CARGO_MANIFEST_DIR` or relative to the source file. The two-branch decision is made in PR-00 and recorded before PR-09 starts: if `CARGO_MANIFEST_DIR`-relative (expected for `wayland-scanner = 0.31.8`), the original strings `"src/protocols/cosmic-workspace-unstable-v1.xml"` are used unchanged; if source-file-relative, PR-09 switches both to `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")` form. Do not treat this as a risk hedge — the resolution strategy is determined in PR-00.

### `mod tests;` and `mod integration_tests;` placement
**Keep them attached to `main.rs`** unchanged. Reasons:
- They currently use `use super::*` against `main.rs`. The single re-export block (above) preserves that contract with zero edits to the test files.
- Moving them under a sibling module would require updating every `use super::*` in 11000+ lines of test code.
- The brief explicitly says “default: leave them alone unless splitting them simplifies a PR.” It does not.

### `Cargo.toml [[bin]].path`
**No change.** `src/daemon/main.rs` remains the entrypoint.

### `build.rs`
Read in full: it does **not** reference `src/daemon/main.rs`. It only reads `src/gnome-extension/*` and writes to `target/{profile}/gnome/`. **No update needed.**

### `flake.nix` / `nix build`
Not inspected line-by-line, but the Cargo.toml `[[bin]].path` is unchanged and no new dependencies are added, so Nix derivation inputs do not change. If the flake pins specific source globs, confirm in PR-00 — but the standard `crane`/`buildRustPackage` setups glob `src/**/*.rs` and need no edit.

---

## 7. Risks and rollbacks

1. **`wayland_scanner` macro path resolution (PR-09)**. PR-00 mandates a compile probe to determine the resolution base (see PR-00 scope). The branch decision (keep original path strings vs switch to `concat!(env!("CARGO_MANIFEST_DIR"), ...)` form) is recorded in PR-00 before PR-09 starts. If the probe result is applied correctly in PR-09, this risk is fully mitigated. **Rollback**: revert PR-09 — the cosmic protocols are isolated to one PR.

2. **`include_str!` relative path in PR-15**. The `gnome_ext_file!` macro embeds files via `"../../src/gnome-extension/..."`. When the macro moves from `src/daemon/main.rs` to `src/daemon/gnome_ext/embed.rs`, the prefix needs an extra `..`. If this is missed, `cargo build --features embed-gnome-extension` (the default feature set) fails. **Mitigation**: the PR description explicitly calls out the path bump; verify with both `cargo build` (default features) and `cargo build --no-default-features`.

3. **Test-mod access to private items via `use super::*`**. The all-at-once `#[cfg(test)] pub(crate) use ...` block in `main.rs` is the load-bearing piece. If any test (especially in `integration_tests.rs`) accesses something the refactor moved but the re-export missed, the test compile fails with “cannot find X in this scope.” **Mitigation**: run `cargo test --no-run` after every PR; the compile error names the missing symbol. Add it to the re-export block in the same PR.

4. **Visibility leaks (general)**. Forgetting `pub(crate)` on a type that another module references → “struct is private” errors. **Mitigation**: default to `pub(crate)` on every moved item from the start; downgrade later if the linter (`dead_code`/`unused`) suggests.

5. **Circular module dependencies**. The required dependency direction is: `supervisor` → `backends` → (shared `env`, `errors`, `broadcasters`, `display_override`); `backends` do not import `supervisor`. **`BackendContext`** lives in `supervisor/mod.rs`, so backends do not consume it; `supervisor` constructs and passes it to backends. **`BackendExit`** and `map_run_outcome_to_backend_exit` live in `backends/mod.rs` (the trait's natural home, next to `FocusBackend`); `supervisor` imports `BackendExit` from `crate::backends` — this is the correct direction. If `BackendExit` were placed in `supervisor/mod.rs`, every backend impl of `FocusBackend::run` would have to `use crate::supervisor::BackendExit`, creating a backends→supervisor edge and violating the invariant. **`resolve_display_override_for_environment`** is consumed by both `supervisor` (lines 3692/3706) AND `backends/mod.rs` via `query_focus_for_env` (lines 2530/2536). To avoid a backends→supervisor dependency, display-override helpers live in the top-level `display_override.rs` (a sibling of both), and both `supervisor/` and `backends/` import from `crate::display_override::*`.

6. **Clippy lint scope differences**. Lints applied at crate root see all modules; lints inside a module file see only that module. If a `#[allow(dead_code)]` was needed at module scope and the refactor moves it deeper, the allow may stop applying. **Mitigation**: run `cargo clippy --all-targets -- -D warnings` after every PR. The `cosmic_workspace`/`cosmic_toplevel` modules already have `#![allow(...)]` inner attributes — those move with the module body and stay scoped correctly.

7. **`pub struct KanataClient` and `pub enum Environment` are already `pub`**. Make sure the refactor keeps them `pub`, not `pub(crate)`. The diff should leave the existing `pub` keyword in place when these items move.

8. **PR-04 size**. ~1140 LOC moved across four destination files (each under 1200 LOC). Splitting into 4a/4b/4c is recommended for reviewability per the PR description. Not a correctness risk.

---

## 8. Out of scope / follow-ups

Things noticed during exploration but intentionally not touched:

1. **`tests.rs` size (4864 lines) and `integration_tests.rs` size (7176 lines)** — clear candidates for their own multi-file split, but unrelated to the daemon refactor goal and would balloon the PR series. Track separately.
2. **`SupervisorState` and `run_lifecycle_supervisor` complexity** — the `_with_starter_and_resolver` variant exists purely as a test seam. After `FocusBackend` lands (PR-12), this seam might be replaceable with a test-mode `FocusBackend` impl, simplifying the supervisor. **Follow-up, not refactor.**
3. **The `fail_fast_lifecycle_monitor` / `expect_*_or_fail_fast` helpers** in `lifecycle/logind.rs` are generic infrastructure that could move to a shared util module, but the only consumers are inside that file. Leave them there.
4. **Duplicate `mod tests;` re-export overhead** — the verbose `pub(crate) use` block is ergonomic debt. A follow-up could replace it with explicit `use` lines inside the test modules. Not in scope.
5. **`run_lifecycle_supervisor_with_starter_and_resolver`** has a six-parameter signature with two generic closures. After `FocusBackend` exists, this could collapse. **Follow-up.**
6. **`KanataClientInner` / `KanataClient` split** — the inner Arc-Mutex pattern is duplicated for several broadcasters. A `SharedState<T>` newtype could DRY it. Not behaviour-preserving in the sense of binary-identity; out of scope.
7. **`SNI_RUNTIME_RETRY_INTERVAL` constant** — located at main.rs line 1708 (between `plan_sni_runtime_transition` at 1688 and `SniRuntimeWakeReason` at 1711). Moves to `sni/guard.rs` in PR-14. Assignment is already listed in the PR-01 constant inventory above.
8. **The `Args` clap derive holds CLI default values inline.** Could be extracted to constants in `args.rs`. Not behaviour-changing, but ergonomics > 0 — track as follow-up.

---

# Summary (per request)

**PR count**: 15 PRs (PR-00 through PR-15), with PR-04, PR-08, and PR-14 explicitly splittable into 2–3 sub-PRs at the executor’s discretion, putting the realistic upper bound at ~18.

**Final `main.rs` target LOC**: 250–400 lines, containing only the `use` block, ~20 `mod` declarations, the `#[cfg(test)] pub(crate) use` re-export block for test access, `#[tokio::main] async fn main`, `async fn run_once`, and the `#[cfg(test)] mod tests; #[cfg(test)] mod integration_tests;` lines.

**Trait seams I’d actually use**: one new — `FocusBackend` (turns the five `run_*_backend_task` adapter functions into a `Box<dyn FocusBackend>` dispatch in the supervisor and gives integration tests a clean injection point; lands in PR-12). Two existing — `DconfBackend` and `SniControlOps` — kept as-is, but made the consumed type at every call site (already true today; just move them into `sni/`). Explicitly **not** turning `LifecycleProvider` (two-variant enum, distinct semantics) or `KanataClient` (single impl, no test double) into traits — overengineering per the global CLAUDE.md.

**Risk to flag to the user**: two land-mines for the executor — (1) the `wayland_scanner::generate_*` macro path resolution behavior on the pinned `wayland-scanner = 0.31.8` (almost certainly `CARGO_MANIFEST_DIR`-relative, so the move-as-is works, but PR-00 must verify); and (2) the `include_str!`-via-`gnome_ext_file!` relative path needs an extra `..` segment when the macro definition moves from `main.rs` to `gnome_ext/embed.rs` (PR-15). Both are caught by `cargo build` with the default feature set — but if they surface late in a PR series, they will look mysterious.