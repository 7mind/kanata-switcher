# Test file split plan (M2)

Date: 2026-05-13
Branch: trait-refactor
Status: draft

## 1. Goals and non-goals

### Goals

- Split `src/daemon/tests.rs` (4864 LOC, 189 unit tests) into a directory module
  under `src/daemon/tests/`, grouped by subsystem.
- Split `src/daemon/integration_tests.rs` (7176 LOC, 65 tests) into a directory
  module under `src/daemon/integration_tests/`, grouped by backend.
- Operational success criteria:
  - `cargo test --bin kanata-switcher -- --test-threads=4` reports exactly the
    same `261 passed / 0 failed` before and after every PR.
  - No test renaming, no test merging, no test deletion.
  - Test attributes (`#[test]`, `#[tokio::test(...)]`, `#[ignore]`,
    `proptest!{...}`) preserved byte-identical.
  - Soft LOC budget: target < 1000 LOC per leaf test file. Strict ceiling 1500
    LOC. Split larger sections; do not split a single test across files.
  - Each PR builds and tests green; each PR is independently revertable.

### Non-goals

- No daemon code changes (production files untouched except possibly the
  `#[cfg(test)] pub(crate) use crate::{...}` re-export block in `main.rs`, only
  if explicitly required — see §5).
- No `Cargo.toml` changes. `[[bin]].path` stays `src/daemon/main.rs`. The
  attachment lines `#[cfg(test)] mod tests;` and `mod integration_tests;` in
  `main.rs` are kept.
- No new dependencies, no version bumps.
- No flake fixes for X11 tests, no mock improvements, no test additions.
- "By platform (linux)" reduces to "by backend / by subsystem". The entire
  daemon is Linux-only; introducing a `linux/` subdirectory adds depth without
  value. **Interpretation: group by backend.** If the user disagrees, we can
  add a thin `linux/` indirection layer on top of the proposed tree without
  changing leaf-file content.

## 2. Current inventory

Numbers are based on `grep` over the two files at HEAD on `trait-refactor`.

### `src/daemon/tests.rs` (4864 LOC, 189 `#[test]`/`#[tokio::test]` attrs)

Top-of-file (lines 1–111):
- `use super::*;` plus 9 `use ...;` lines (lines 1–10).
- Constants: `TEST_TIMEOUT` (12), `SNI_WATCHER_TEST_LOCK: Mutex<()>` (13).
- Helpers: `with_test_timeout` (15), `win` (24), `rule` (32), `rule_vk` (44),
  `rule_raw_vk` (56), `rule_with_fallthrough` (73), `has_action` (79),
  `get_layers` (84), `get_raw_vk_actions` (98).

Section markers and contents:

| Marker (line) | LOC | Tests | Subject |
|---|---|---|---|
| `// === Flow Tests ===` (112) | 1761 | 90 tests + 1 `proptest!` block (line 497, 1 prop test) | mixed: focus flow, autostart, KWin script paths, dbus suffix sanitisation, control commands, SNI indicator/menu/settings, virtual key press/release, fallthrough, wildcard/regex, raw_vk_action. Subdivisions visible by helper-cluster: |
| └── focus flow (115–263) | | 8 | basic match, no-match, same-window, title change, unfocus VKs, native-terminal |
| └── autostart (266–321) | | 3 | passthrough args, exec escaping |
| └── kde script paths (323–392) | | 4 | KWin query/probe/runtime script paths |
| └── dbus_naming (395–712) | | 28 + 1 prop | `derive_default_dbus_suffix`, `sanitize_dbus_suffix`, `effective_dbus_name`, `is_daemon_bus_name`, `Args.dbus_suffix`, KDE focus push script; one `proptest!` (497–558) for `prop_sanitize_dbus_suffix_invariants` |
| └── control commands (714–742) | | 4 | `resolve_control_command` mapping |
| └── SNI presentation (744–1224) | | 14 | format layer letter, virtual keys, icon color, indicator state, settings store (dconf-mock), toggle persist, menu actions, tooltip, title text |
| └── focus pipeline async (1226–1335) | | 4 | `update_status_for_focus`, paused handling |
| └── virtual keys / fallthrough (1336–1872) | | 24 | press/release on focus, partial set, unfocus order, raw_vk_action, fallthrough chain, wildcard, regex, non-fallthrough stops |
| `// === Property Tests ===` (1873) | 277 | 5 prop tests in 1 `proptest!` macro (1958–2148) | proptest strategies (`arb_class`, `arb_title`, etc.) + 5 invariants on FocusHandler |
| `// === GNOME Extension State Parsing Tests ===` (2150) | 308 | 18 tests | **misnamed** — actually contains: (a) 3 gnome-ext state tests (2152–2193); (b) 3 `wait_for_restart_or_shutdown` lifecycle tests (2195–2239); (c) 9 logind object-path / error-detection tests (2241–2374); (d) 2 `update_status_for_focus` filtering tests (2376–2456) |
| `// === Config Parsing Tests ===` (2458) | 118 | 8 tests | `serde_json::from_str::<Vec<ConfigEntry>>` validation |
| `// === Runtime Lifecycle Tests ===` (2576) | 2289 | 73 tests | further subdivided below |

Runtime Lifecycle Tests subgroups (driven by test name prefixes; line ranges
approximate):

- helpers (2578–2642): `test_backend_context*`, `test_running_backend_handle`,
  `test_finished_backend_handle` — **these are helpers, not tests** despite
  the `test_` prefix.
- session/logind decoding (2644–2895): `test_session_type_to_session_kind*`,
  `test_validate_active_logind_session_type*`, `test_decode_logind_change_*`,
  `test_decode_logind_display_path_change_*` (~14 tests)
- display path application (2897–2976): `test_apply_logind_display_change_*`
  (~5 tests)
- expect/fail-fast (2978–2993): 2 tests
- runtime target / startup snapshot (2995–3170):
  `test_resolve_runtime_target_matrix`, `test_startup_snapshot_*`,
  `test_target_requires_session_bus`, `test_startup_environment_to_snapshot_mapping`,
  `test_runtime_target_label_is_stable`, `test_runtime_target_to_environment_mapping`
  (~7 tests)
- persistent DBus reconnect (3171–3243): `test_dbus_reconnect_delay_caps_at_two_seconds`,
  `test_wait_for_dbus_reconnect_retry_backoffs_for_name_lost_monitor_setup_errors`
  (2 tests)
- SNI runtime transitions (3244–3540): `test_sni_control_mode_tracks_*`,
  `test_plan_sni_runtime_transition_*`, `test_sni_local_control_*`,
  `test_sni_runtime_managed_*`, `test_map_run_outcome_to_backend_exit`
  (~7 tests)
- transition runtime target / desktop sequencing (3554–4080):
  `test_transition_runtime_target_*`, `test_stop_current_backend_with_no_backend_is_noop`
  (~7 tests)
- lifecycle_provider / resolver (3686–3879):
  `test_resolve_desktop_flavor_wayland_precedence`,
  `test_lifecycle_provider_*`, `test_resolve_runtime_target_for_*`,
  `test_startup_snapshot_provider_emits_once` (~8 tests)
- run_lifecycle_supervisor (4098–end): `test_run_lifecycle_supervisor_*`,
  `test_poll_finished_backend_outcome_*` (~13 tests)

### `src/daemon/integration_tests.rs` (7176 LOC, 65 `#[tokio::test]`/`#[test]` attrs)

Top-of-file (lines 1–19): `use super::*;` and stdlib imports.

| Marker (line) | LOC | Tests | Subject | Notes |
|---|---|---|---|---|
| `// === Polling Helper ===` (20) | 164 | 0 (infrastructure) | constants (`POLL_INTERVAL`, `POLL_TIMEOUT`, `TEST_TIMEOUT`, `LONG_TEST_TIMEOUT`), `WAYLAND_ENV_LOCK`, `DBUS_ENV_LOCK`, `X11_FOCUS_QUERY_LOCK`, `DISPLAY_ENV_LOCK`; `EnvVarGuard`; `wait_for`/`wait_for_async`/`with_test_timeout`/`with_long_test_timeout`; `start_wayland_test_server`; `pause_daemon_direct`, `unpause_daemon_direct` | shared across all suites |
| `// === Mock Kanata Server ===` (184) | 259 | 0 (infrastructure) | `KanataMessage` enum, `FocusService` zbus interface, `TEST_DAEMON_DBUS_NAME` const, `start_gnome_focus_service`, `wait_for_kanata_message`, `drain_kanata_messages`, `MockKanataConfig`, `MockKanataServer` (incl. `start`, `start_legacy`, `start_with_config`) | shared widely |
| `// === GNOME Focus Query Tests ===` (443) | 99 | 1 | `test_gnome_focus_query_on_start_and_unpause` | needs `start_gnome_focus_service`, `DbusSessionGuard` |
| `// === KDE Focus Query Tests ===` (542) | 740 | 4 | `MockKwinScripting`, `MockKwinScript`, `extract_call_dbus_parts` (helper used only here), then `test_kde_focus_query_on_start_and_unpause`, `test_dbus_unpause_resolves_kde_runtime_mode_without_startup_env`, `test_run_kde_resolves_runtime_mode_without_startup_env`, `test_run_kde_waits_for_scripting_interface_before_runtime_probe` | needs `DbusSessionGuard`, `MockKanataServer` |
| `// === DBus Integration Tests ===` (1283) | 287 | 3 | `test_dbus_service_layer_switching`, `test_dbus_service_virtual_keys`, `test_dbus_service_fallthrough` — these test the control DBus surface against in-process registration | no private session bus needed; uses `MockKanataServer` only |
| `// === Private DBus Session for Testing ===` (1570) | 1750 | 14 | infrastructure (1570–1689): `dbus_daemon_available`, `DBUS_TEST_COUNTER`, `DbusSessionGuard`. Tests (1699–3319): `test_dbus_service_real_bus`, `test_dbus_get_status_initial_layer`, `test_dbus_get_status_focus_source`, `test_dbus_restart_request`, `test_control_command_restart_private_dbus`, `test_dbus_pause_unpause`, `test_dbus_paused_changed_signal`, `test_dbus_status_changed_focus_signal`, `test_handle_focus_event_ignored_when_paused`, `test_dbus_pause_wayland_env`, `test_control_command_returns_error_without_service`, `test_unfocus_ignored_when_paused`, `test_pause_daemon_releases_virtual_keys_and_resets_layer`, `test_control_command_pause_unpause_private_dbus`, `test_persistent_dbus_service_handles_restart_in_idle_runtime` | infrastructure → `common/dbus_session.rs`; tests → `dbus_control.rs` + `dbus_pause.rs` + `dbus_persistent.rs` |
| `// === Wayland Protocol Integration Tests ===` (3320) | 443 | 4 | `mod wayland_mock { ... }` submodule (3327–3560) with `MockCompositorState`, `WaylandMockServer`; then 2 multi-thread tokio tests and 2 sync tests | `mod wayland_mock` stays as inline child module of the new `wayland.rs` |
| `// === X11/Xvfb Integration Tests ===` (3763) | 752 | 6 | `xvfb_available`, `XvfbGuard`, then `test_x11_state_display_override_ignores_stale_display_env`, `test_x11_property_notify`, `test_x11_focus_handler_integration`, `test_x11_multiple_focus_changes`, `test_x11_focus_query_on_start_and_unpause`, `test_x11_unpause_focus_query_uses_runtime_display_override` | uses hardcoded Xvfb displays `:100`/`:101`/`:102`/`:104`. Note: nextest isolates each test in its own process so file-grouping is irrelevant to display contention. |
| `// === GNOME Shell Extension Detection Integration Tests ===` (4515) | 308 | 2 | `MockGnomeShellExtensions`, `test_gnome_extension_dbus_probe_integration`, `test_gnome_extension_delayed_activation` | needs `DbusSessionGuard` |
| `// === Virtual Key Validation Tests ===` (4823) | 693 | 12 | `test_virtual_key_validation_*`, `test_handshake_requests_fake_key_names`, `test_invalid_vk_*`, `test_raw_vk_action_mixed_valid_invalid`, etc. | uses `MockKanataServer` (incl. `start_legacy`) |
| `// === dconf Integration Tests ===` (5516) | 200 | 3 | `is_dconf_available`, `IsolatedDconfEnv`, then `test_dconf_write_and_read_bool_isolated`, `test_dconf_read_unset_key_isolated`, `test_sni_settings_store_with_isolated_dconf` | self-contained; helper used only here |
| `// === DBus multi-instance integration tests ===` (5716) | 1460 | 15 | `TEST_DAEMON_DBUS_NAME_A`/`_B` consts, `register_test_daemon_with_name`, then 14 multi-instance tests (`test_two_daemons_register_independent_names`, `test_control_command_targets_specific_daemon_when_suffix_given`, etc.) plus `test_kde_focus_push_script_targets_per_instance_name` (sync unit test, line 7146 — moves with multiplex) | needs `DbusSessionGuard`, `MockKanataServer` |

## 3. Target module tree

### `src/daemon/tests/`

```
src/daemon/tests/
├── mod.rs                          # mod declarations; re-export common helpers; `use super::*;`
├── common/
│   ├── mod.rs                      # pub(super) use helpers + re-export TEST_TIMEOUT, SNI_WATCHER_TEST_LOCK
│   └── helpers.rs                  # fn win, rule, rule_vk, rule_raw_vk, rule_with_fallthrough, has_action, get_layers, get_raw_vk_actions, with_test_timeout
├── focus_flow.rs                   # focus flow + virtual keys + fallthrough + patterns (~860 LOC, ~32 tests)
├── focus_pipeline.rs               # update_status_for_focus (sync + async, incl. filter VK tests at 2376–2456) (~250 LOC, 6 tests)
├── focus_property.rs               # `proptest!` block at 1958–2148 + strategies `arb_class` etc. (~280 LOC, 5 prop tests)
├── autostart.rs                    # 3 autostart tests (266–321) (~60 LOC)
├── dbus_naming.rs                  # 28 dbus_naming/suffix/Args tests + 1 `proptest!` (497–558) (~620 LOC)
├── control_commands.rs             # 4 resolve_control_command tests (~30 LOC)
├── kde_script_paths.rs             # 4 KWin script-path tests + test_build_kde_focus_push_script_targets_instance_name (~210 LOC)
├── sni_presentation.rs             # 14 SNI format/icon/state/settings/menu/tooltip/title tests (~480 LOC)
├── gnome_ext_state.rs              # 3 gnome-extension-state parsing tests (~50 LOC)
├── config_parsing.rs               # 8 config parsing tests (~120 LOC)
└── lifecycle/
    ├── mod.rs                      # mod decls; shared backend-context fixtures
    ├── fixtures.rs                 # test_backend_context, test_backend_context_with_gnome_setup, test_running_backend_handle, test_finished_backend_handle
    ├── restart_or_shutdown.rs      # 3 wait_for_restart_or_shutdown tests
    ├── logind_decode.rs            # 9 logind path/error tests + 14 logind change decode/display-path tests (~770 LOC, ~23 tests)
    ├── display_apply.rs            # 5 apply_logind_display_change tests (~100 LOC)
    ├── runtime_target.rs           # resolve_runtime_target_matrix + startup_snapshot + target_requires_session_bus + runtime_target_label/mapping + resolve_desktop_flavor_wayland_precedence + resolve_runtime_target_for_* (~600 LOC, ~10 tests)
    ├── persistent_dbus.rs          # 2 persistent-dbus reconnect tests
    ├── sni_runtime.rs              # 7 SNI runtime/control-mode/local-control/managed tests (~370 LOC)
    ├── transition.rs               # 7 transition_runtime_target_* tests + stop_current_backend_with_no_backend (~580 LOC)
    ├── provider.rs                 # 4 lifecycle_provider tests + startup_snapshot_provider_emits_once + map_run_outcome_to_backend_exit (~250 LOC)
    └── supervisor.rs               # 13 run_lifecycle_supervisor_* + poll_finished_backend_outcome (~750 LOC)
```

Worst case: `lifecycle/supervisor.rs` ≈ 750 LOC; `lifecycle/logind_decode.rs`
≈ 770 LOC; `dbus_naming.rs` ≈ 620 LOC; `lifecycle/runtime_target.rs` ≈ 600
LOC; `lifecycle/transition.rs` ≈ 580 LOC. All within budget.

### `src/daemon/integration_tests/`

```
src/daemon/integration_tests/
├── mod.rs                          # mod declarations; `use super::*;`; pub use common::*
├── common/
│   ├── mod.rs                      # pub(super) use
│   ├── polling.rs                  # consts POLL_INTERVAL, POLL_TIMEOUT, TEST_TIMEOUT, LONG_TEST_TIMEOUT; statics WAYLAND_ENV_LOCK, DBUS_ENV_LOCK, X11_FOCUS_QUERY_LOCK, DISPLAY_ENV_LOCK; EnvVarGuard; wait_for, wait_for_async, with_test_timeout, with_long_test_timeout
│   ├── mock_kanata.rs              # KanataMessage enum, MockKanataConfig, MockKanataServer (incl. start_legacy), wait_for_kanata_message, drain_kanata_messages
│   ├── focus_service.rs            # FocusService zbus interface; TEST_DAEMON_DBUS_NAME const; start_gnome_focus_service; pause_daemon_direct, unpause_daemon_direct; start_wayland_test_server
│   └── dbus_session.rs             # dbus_daemon_available, DBUS_TEST_COUNTER, DbusSessionGuard
├── gnome/
│   ├── mod.rs
│   ├── focus_query.rs              # 1 test (~99 LOC)
│   └── extension_detection.rs      # MockGnomeShellExtensions + 2 tests (~308 LOC)
├── kde/
│   ├── mod.rs
│   └── focus_query.rs              # MockKwinScripting, MockKwinScript, extract_call_dbus_parts + 4 tests (~740 LOC)
├── wayland.rs                      # mod wayland_mock { ... } inline child + 4 tests (~443 LOC)
├── x11.rs                          # xvfb_available, XvfbGuard + 6 tests (~752 LOC)
├── vk_validation.rs                # 12 tests (~693 LOC)
├── dbus_control.rs                 # 3 tests from "DBus Integration Tests" (1283–1569) (~287 LOC)
├── dbus_session_tests.rs           # 14 tests from "Private DBus Session" (1699–3319); split further if it exceeds 1500 LOC (~1450 LOC)
├── dbus_multiplex.rs               # const TEST_DAEMON_DBUS_NAME_A/_B, register_test_daemon_with_name, 14 multiplex tests + test_kde_focus_push_script_targets_per_instance_name (~1460 LOC)
└── dconf.rs                        # IsolatedDconfEnv + 3 tests (~200 LOC)
```

If `dbus_session_tests.rs` ends up > 1500 LOC, split into:
- `dbus_session_status.rs` (status/signal/restart: ~6 tests)
- `dbus_session_pause.rs` (pause/unpause/paused-ignored/release-vks: ~6 tests)
- `dbus_session_persistent.rs` (persistent-service handles restart in idle:
  1 test) + `dbus_session_real_bus.rs` (the bare real-bus test).

I leave that as a follow-up only if the actual byte count blows the budget.
Initial cut keeps them together.

## 4. Shared helpers inventory

### `tests.rs` helpers (all near top of file)

| Helper | Current location | Visibility | Destination |
|---|---|---|---|
| `const TEST_TIMEOUT: Duration` | tests.rs:12 | private | `tests/common/helpers.rs` as `pub(super) const` |
| `static SNI_WATCHER_TEST_LOCK: Mutex<()>` | tests.rs:13 | private | `tests/common/helpers.rs` as `pub(super) static` (used in SNI runtime tests) |
| `with_test_timeout` | tests.rs:15 | `async fn` private | `tests/common/helpers.rs` as `pub(super) async fn` |
| `win` | tests.rs:24 | `fn` private | `tests/common/helpers.rs` as `pub(super) fn` |
| `rule` | tests.rs:32 | private | `tests/common/helpers.rs` `pub(super)` |
| `rule_vk` | tests.rs:44 | private | `tests/common/helpers.rs` `pub(super)` |
| `rule_raw_vk` | tests.rs:56 | private | `tests/common/helpers.rs` `pub(super)` |
| `rule_with_fallthrough` | tests.rs:73 | private | `tests/common/helpers.rs` `pub(super)` |
| `has_action` | tests.rs:79 | private | `tests/common/helpers.rs` `pub(super)` |
| `get_layers` | tests.rs:84 | private | `tests/common/helpers.rs` `pub(super)` |
| `get_raw_vk_actions` | tests.rs:98 | private | `tests/common/helpers.rs` `pub(super)` |
| `test_backend_context_with_gnome_setup<F>` | tests.rs:2578 | private | `tests/lifecycle/fixtures.rs` `pub(super)` |
| `test_backend_context` | tests.rs:2603 | private | `tests/lifecycle/fixtures.rs` `pub(super)` |
| `test_running_backend_handle` | tests.rs:2607 | private | `tests/lifecycle/fixtures.rs` `pub(super)` |
| `test_finished_backend_handle` | tests.rs:2629 | private | `tests/lifecycle/fixtures.rs` `pub(super)` |
| proptest strategies `arb_class`, `arb_nonempty_class`, `arb_title`, `arb_layer`, `arb_vk_name`, `arb_vk_action`, `arb_rule`, `arb_window` | tests.rs:1875–1955 | private | `tests/focus_property.rs` (used only there; keep file-local) |

### `integration_tests.rs` helpers

| Helper | Current location | Visibility | Destination |
|---|---|---|---|
| `POLL_INTERVAL`, `POLL_TIMEOUT`, `TEST_TIMEOUT`, `LONG_TEST_TIMEOUT` | 22–25 | private const | `integration_tests/common/polling.rs` `pub(super) const` |
| `WAYLAND_ENV_LOCK`, `DBUS_ENV_LOCK`, `X11_FOCUS_QUERY_LOCK`, `DISPLAY_ENV_LOCK` | 26–29 | private static `Mutex<()>` | `integration_tests/common/polling.rs` `pub(super) static` |
| `EnvVarGuard` struct + impls | 31–57 | private | `common/polling.rs` `pub(super)` |
| `wait_for`, `wait_for_async` | 61, 76 | private | `common/polling.rs` `pub(super)` |
| `with_test_timeout`, `with_long_test_timeout` | 91, 100 | private | `common/polling.rs` `pub(super)` |
| `start_wayland_test_server` | 109 | private | `common/focus_service.rs` `pub(super)` (only Wayland tests use it; if Wayland is the only consumer, move there instead — confirm during inventory pass in PR-1) |
| `pause_daemon_direct`, `unpause_daemon_direct` | 118, 150 | private | `common/focus_service.rs` `pub(super)` |
| `KanataMessage` enum | 188 | private | `common/mock_kanata.rs` `pub(super)` |
| `FocusService` + zbus impl | 195 | private | `common/focus_service.rs` `pub(super)` |
| `TEST_DAEMON_DBUS_NAME` const | 222 | private | `common/focus_service.rs` `pub(super)` |
| `start_gnome_focus_service` | 224 | private | `common/focus_service.rs` `pub(super)` |
| `wait_for_kanata_message`, `drain_kanata_messages` | 268, 284 | private | `common/mock_kanata.rs` `pub(super)` |
| `MockKanataConfig` + `Default` impl | 293 | private | `common/mock_kanata.rs` `pub(super)` |
| `MockKanataServer` + impls | 311 | private | `common/mock_kanata.rs` `pub(super)` |
| `dbus_daemon_available` | 1573 | private | `common/dbus_session.rs` `pub(super)` |
| `DBUS_TEST_COUNTER` (and `use std::sync::atomic::{AtomicU64,Ordering}` at 1588) | 1589 | private static | `common/dbus_session.rs` `pub(super)` |
| `DbusSessionGuard` + impls | 1582 | private | `common/dbus_session.rs` `pub(super)` |
| `MockKwinScripting`, `MockKwinScript`, `extract_call_dbus_parts` | 544, 587, 640 | private | `kde/focus_query.rs` (only consumer) — keep file-local |
| `mod wayland_mock {...}` | 3327 | private inline module | `wayland.rs` — keep as inline child module |
| `xvfb_available`, `XvfbGuard` | 3766, 3775 | private | `x11.rs` (only consumer) — keep file-local |
| `MockGnomeShellExtensions` | 4519 | private | `gnome/extension_detection.rs` (only consumer) |
| `is_dconf_available`, `IsolatedDconfEnv` + impls + `DCONF_TEST_KEY` const | 5518, 5520, 5532 | private | `dconf.rs` (only consumer) |
| `TEST_DAEMON_DBUS_NAME_A`/`_B` | 5718–5719 | private const | `dbus_multiplex.rs` (only consumer) |
| `register_test_daemon_with_name` | 5721 | private | `dbus_multiplex.rs` (only consumer) |

### Test-only items in production code (verify only — no changes)

- `pause::TEST_LAST_UNPAUSE_REQUEST_ENV` — used by some pause/unpause tests.
  Confirm it is `#[cfg(test)] pub(crate)` in `src/daemon/pause.rs`; if so, it
  is already reachable from the new test files via the existing `#[cfg(test)]
  pub(crate) use crate::{pause::*}` re-export in `main.rs:64`.
- `sni::indicator::ACTIVE_SNI_WATCHER_TASKS` — used by
  `test_sni_runtime_managed_transitions_do_not_leak_watcher_tasks`. Confirm
  the same.
- `kanata::KanataClient::inner` field — used by
  `test_update_status_for_focus_filters_invalid_virtual_keys` via
  `kanata.inner.try_lock()`. This requires `inner` to be `pub(crate)` (under
  cfg(test) if not already). Verify; if not, this is the one daemon-side
  visibility tweak required. Document, then if needed bundle into PR-0.

## 5. Path-resolution strategy

**Recommend Option A: directory modules with `use super::*;`.**

Rationale: Rust's module system makes `mod tests;` resolve to *either*
`tests.rs` or `tests/mod.rs`. Switching to `tests/mod.rs` is the standard
Cargo idiom and is a one-line attachment change in `main.rs` (only the
`mod tests;` line stays put; we just delete the old file and create a
directory in its place). The `super::*` chain through nested submodules
inherits the existing re-export block at `main.rs:64`. Total daemon-side
edits: zero (except verifying the `KanataClient::inner` visibility item from
§4).

Option B (move re-exports to crate root) would require either:
- Making `pub(crate) use crate::{...}` unconditional (production code now
  imports its own helpers through the same paths — possible but a larger
  surface change), or
- Duplicating the re-export under a `#[cfg(test)]` block at crate root and
  letting nested modules `use crate::*` — equivalent surface change.

Option A wins on diff size. Pick A. If a blocker surfaces during PR-1
(e.g., a test references a name like `pause::*` not covered by the existing
re-export), the fix is local: extend the re-export block in `main.rs:64`,
not change the strategy.

### Boilerplate

#### `src/daemon/main.rs` (unchanged)

```rust
#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;
```

The `mod tests;` attribute resolves to `tests/mod.rs` after the split (Rust
resolution rules pick whichever exists; we'll delete `tests.rs` in the same
commit). The `#[cfg(test)] pub(crate) use crate::{...}` block at line 64 stays.

#### `src/daemon/tests/mod.rs`

```rust
use super::*;            // pulls everything re-exported by the crate::{...} block
                         // in main.rs

mod common;
pub(super) use common::*;  // not strictly necessary but lets leaf files use them
                            // via `use super::*;` rather than `use super::common::*;`

mod autostart;
mod config_parsing;
mod control_commands;
mod dbus_naming;
mod focus_flow;
mod focus_pipeline;
mod focus_property;
mod gnome_ext_state;
mod kde_script_paths;
mod sni_presentation;
mod lifecycle;
```

#### `src/daemon/tests/common/mod.rs`

```rust
use super::*;            // re-import daemon names so helpers compile

mod helpers;
pub(super) use helpers::*;
```

#### `src/daemon/tests/common/helpers.rs`

```rust
use super::*;            // daemon names

pub(super) const TEST_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

pub(super) static SNI_WATCHER_TEST_LOCK: std::sync::Mutex<()> =
    std::sync::Mutex::new(());

pub(super) async fn with_test_timeout<F, T>(future: F) -> T
where F: std::future::Future<Output = T>,
{ tokio::time::timeout(TEST_TIMEOUT, future).await.expect("test timeout") }

pub(super) fn win(class: &str, title: &str) -> WindowInfo { /* unchanged body */ }

pub(super) fn rule(...) -> Rule { /* unchanged */ }
// ... rest of helpers
```

#### `src/daemon/tests/focus_flow.rs`

```rust
use super::*;            // pulls daemon names AND helpers re-exported by mod.rs

use proptest::prelude::*; // only if needed
// any other imports the original file needed (clap::Parser, etc.)

#[test]
fn test_basic_layer_match() {
    // unchanged body — `win`, `rule`, etc. resolve via super::*
}
// ...
```

#### `src/daemon/tests/lifecycle/mod.rs`

```rust
use super::*;            // daemon names + tests/ common helpers

mod fixtures;
pub(super) use fixtures::*;

mod restart_or_shutdown;
mod logind_decode;
mod display_apply;
mod runtime_target;
mod persistent_dbus;
mod sni_runtime;
mod transition;
mod provider;
mod supervisor;
```

#### `src/daemon/tests/lifecycle/fixtures.rs`

```rust
use super::*;
// re-exported by mod.rs via `pub(super) use fixtures::*`

pub(super) fn test_backend_context() -> BackendContext { /* unchanged */ }
// ... others
```

#### Walkthrough

A test in `tests/focus_flow.rs` calls `FocusHandler::new(...)`.

1. `FocusHandler` is referenced in `focus_flow.rs`.
2. `focus_flow.rs` has `use super::*;`. `super` here is the `tests` module
   (`src/daemon/tests/mod.rs`).
3. `tests/mod.rs` has `use super::*;`. `super` here is the `daemon` binary
   crate root (`src/daemon/main.rs`).
4. `main.rs` has, *under `#[cfg(test)]`*, `pub(crate) use crate::{... focus::*
   ...};` at line 64. Inside the test cfg, `FocusHandler` is therefore
   re-exported as `crate::FocusHandler` *and* visible via the `use super::*`
   glob in step 3.
5. Resolution succeeds: `focus_flow.rs::FocusHandler` →
   `tests::FocusHandler` → `crate::FocusHandler` → `crate::focus::FocusHandler`.

The same chain works for everything currently named in the re-export block.

#### Integration-tests mirror

Identical structure; `src/daemon/integration_tests/mod.rs`:

```rust
use super::*;
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

mod common;
pub(super) use common::*;

mod gnome;
mod kde;
mod wayland;
mod x11;
mod vk_validation;
mod dbus_control;
mod dbus_session_tests;
mod dbus_multiplex;
mod dconf;
```

Leaf files use `use super::*;` plus any per-file-only imports.

## 6. PR breakdown

Each PR builds, runs `cargo test --bin kanata-switcher`, must show
`261 passed / 0 failed`.

### PR-1: Convert `tests.rs` to directory module; extract `common/helpers.rs`

Scope: create `tests/mod.rs`, `tests/common/mod.rs`, `tests/common/helpers.rs`;
delete `tests.rs`. All 189 tests move temporarily into `tests/mod.rs` (one
big file) — i.e. just the rename/relocation step. The 11 helpers and
constants move to `common/helpers.rs`, with `use super::common::*;` (or `use
super::*;` if `pub(super) use common::*;` lands in `mod.rs`) added at the
top of `mod.rs` content.

If preferred, split this PR further: PR-1a moves the file as-is into the
directory; PR-1b extracts helpers. They are independent.

LOC moved: 4864 (relocate) + 11 helper signatures touched (`fn` → `pub(super)
fn`).

Imports: helpers' bodies unchanged; only `pub(super)` visibility added.

Verification: `cargo build --bin kanata-switcher && cargo test --bin
kanata-switcher -- --test-threads=4`. Expect 261/0.

Risk: forgetting `pub(super)` on `SNI_WATCHER_TEST_LOCK` or on one of the
`rule_*` helpers. Caught by `cargo build`.

Rollback: revert the commit.

### PR-2: Split `tests/mod.rs` by top-level section

Scope: extract `focus_flow.rs`, `focus_pipeline.rs`, `focus_property.rs`,
`autostart.rs`, `dbus_naming.rs`, `control_commands.rs`,
`kde_script_paths.rs`, `sni_presentation.rs`, `gnome_ext_state.rs`,
`config_parsing.rs` from the mega-module. Leave the "Runtime Lifecycle
Tests" block in `mod.rs` (or temporarily under `mod lifecycle_inline;`).

LOC moved: ~2575 (everything outside Runtime Lifecycle).

Imports: each new file: `use super::*;` at top; copy any per-file `use`
statements (e.g. `use clap::Parser;` in `dbus_naming.rs` if some test needs
it; `proptest::prelude::*` in `focus_property.rs` and `dbus_naming.rs`).

Verification: `cargo test`. Expect 261/0. Run with `-- --nocapture
test_basic_layer_match` to confirm a specific test still resolves and runs.

Risk: forgetting to bring along a `use zbus::Message;` style import that
only one test needs. Compile error catches it.

Rollback: revert the commit.

### PR-3: Split Runtime Lifecycle into `tests/lifecycle/` subtree

Scope: extract `lifecycle/fixtures.rs`, `restart_or_shutdown.rs`,
`logind_decode.rs`, `display_apply.rs`, `runtime_target.rs`,
`persistent_dbus.rs`, `sni_runtime.rs`, `transition.rs`, `provider.rs`,
`supervisor.rs`. Move the 4 backend-context helpers (`test_backend_context*`,
`test_running_backend_handle`, `test_finished_backend_handle`) to
`lifecycle/fixtures.rs`.

The 3 `wait_for_restart_or_shutdown_*` tests and the 9 logind path/error
tests that were misfiled under the "GNOME Extension State Parsing Tests"
banner relocate to `lifecycle/restart_or_shutdown.rs` and
`lifecycle/logind_decode.rs` respectively. The 2 misfiled
`update_status_for_focus_*` tests relocate to `focus_pipeline.rs`.

LOC moved: ~2289 + the ~470 misfiled lines from PR-2's `gnome_ext_state.rs`.

Verification: `cargo test`. Expect 261/0.

Risk: `lifecycle::fixtures` helpers need `pub(super)` and the file must
`pub(super) use fixtures::*;` in `lifecycle/mod.rs` so its sibling submodules
can call them via `use super::*;`. Forgetting this triggers compile errors.

Rollback: revert the commit.

### PR-4: Convert `integration_tests.rs` to directory module; extract `common/` infrastructure

Scope: create `integration_tests/mod.rs`, `integration_tests/common/mod.rs`,
and the four common files (`polling.rs`, `mock_kanata.rs`,
`focus_service.rs`, `dbus_session.rs`). Move the infrastructure blocks
(lines 22–182, 188–441, 1573–1689) into the common modules. Delete
`integration_tests.rs`; the remaining 65 tests temporarily stay as a single
flat `integration_tests/mod.rs` body (or under a single `mod _tests;`).

LOC moved: ~2173 (infrastructure relocations) + 7176 (file relocation).

Verification: `cargo test --bin kanata-switcher`. Expect 261/0.

Risk: `MockKwinScripting` (KDE-only) and `MockGnomeShellExtensions`
(GNOME-only) and `mod wayland_mock` (Wayland-only) — these are *not* shared;
they should stay file-local in their respective leaf files in PR-5. In PR-4
they stay where they are.

Also: `start_wayland_test_server` references `WAYLAND_ENV_LOCK` (in
`common/polling.rs`) and `wayland_mock::WaylandMockServer` (in the
yet-unmoved Wayland tests). Resolution: keep `start_wayland_test_server` in
`common/focus_service.rs` and pull `pub(super) use
super::super::wayland::wayland_mock::WaylandMockServer` — but this creates a
cycle (`common` referring to `wayland`). Alternative: move
`start_wayland_test_server` and `WAYLAND_ENV_LOCK` to `wayland.rs` itself in
PR-5. Recommended: leave it in PR-4's `common/` as a thin wrapper that
returns the lock guard only, and have the Wayland test file construct the
`WaylandMockServer` itself. PR-4 verification will surface the exact
breakage; design the helper boundary against what compiles.

Rollback: revert the commit.

### PR-5: Split integration tests by backend

Scope: extract `gnome/focus_query.rs`, `gnome/extension_detection.rs`,
`gnome/mod.rs`; `kde/focus_query.rs`, `kde/mod.rs` (carries `MockKwinScripting`,
`MockKwinScript`, `extract_call_dbus_parts` as file-local); `wayland.rs`
(carries `mod wayland_mock`); `x11.rs` (carries `XvfbGuard`); `vk_validation.rs`;
`dbus_control.rs`; `dbus_session_tests.rs`; `dconf.rs` (carries
`IsolatedDconfEnv`); `dbus_multiplex.rs` (carries `register_test_daemon_with_name`
and the per-instance KDE script unit test at 7146).

LOC moved: ~5000.

Verification: `cargo test --bin kanata-switcher`. Expect 261/0. Run X11
tests with `--test-threads=4` to confirm Xvfb display contention behaves the
same (it should; nextest runs per-test processes).

Risk: cross-file import chain mistakes. `dbus_session_tests.rs` needs
`DbusSessionGuard` (from `common/dbus_session.rs`), `MockKanataServer` (from
`common/mock_kanata.rs`), and `pause_daemon_direct`/`unpause_daemon_direct`
(from `common/focus_service.rs`) — all reachable via `use super::*;` if
`mod.rs` does `pub(super) use common::*;`.

Rollback: revert the commit.

### PR-6 (optional): Further-split `dbus_session_tests.rs` if it exceeds budget

Scope: if `dbus_session_tests.rs` lands > 1500 LOC, split into
`dbus_session_status.rs`, `dbus_session_pause.rs`,
`dbus_session_persistent.rs`. Skip if not needed.

### PR-7 (optional): Further-split `focus_flow.rs` if it exceeds budget

Scope: `focus_flow.rs` is projected at ~860 LOC, under budget. But if the
relocated copy lands above 1000 LOC, split into `focus_basic.rs`,
`focus_virtual_keys.rs`, `focus_fallthrough.rs`, `focus_patterns.rs`. Skip
if not needed.

## 7. Cross-cutting concerns

- **Helper duplication across the two modules.** `tests.rs` has its own
  `with_test_timeout` (5s) and `SNI_WATCHER_TEST_LOCK`; `integration_tests.rs`
  has its own `with_test_timeout` (5s) plus a `LONG_TEST_TIMEOUT` (20s) and
  four different env locks. The TWO `with_test_timeout` functions have
  identical bodies but operate on different `TEST_TIMEOUT` constants. **Do
  not merge them** — keep each module's `common/` separate. Merging would
  require a new `src/daemon/test_support/` crate-level module, which is
  outside scope (constraint #9) and outside the user's "tests-only"
  restriction (the helpers would have to live above the `#[cfg(test)] mod
  tests;` attachment). Plan keeps them duplicated, exactly as today.

- **Visibility.** All shared helpers become `pub(super) fn` (or `pub(super)
  const` / `pub(super) static`). `pub(crate)` would be wrong — these are
  test-only and shouldn't pollute the daemon namespace. `pub(super)` is the
  minimum visibility that lets sibling submodules under `tests/` see them
  through the `pub(super) use common::*;` in `tests/mod.rs`.

- **Test-only constants in production code.** PR-0 (folded into PR-1) reads
  `pause.rs`, `sni/indicator.rs`, `kanata.rs` and confirms:
  - `pause::TEST_LAST_UNPAUSE_REQUEST_ENV` is `#[cfg(test)] pub(crate)`.
  - `sni::indicator::ACTIVE_SNI_WATCHER_TASKS` is `#[cfg(test)] pub(crate)`.
  - `kanata::KanataClient::inner` is reachable from tests. The test at
    tests.rs:2399 (`kanata.inner.try_lock()`) requires this. If `inner` is
    private, the one daemon-side edit is to add `#[cfg(test)] pub(crate)` to
    the field declaration. Bundle into PR-1.

- **Mutex serialization.** `SNI_WATCHER_TEST_LOCK` (tests.rs:13) serializes
  SNI runtime-managed tests against each other. `WAYLAND_ENV_LOCK`,
  `DBUS_ENV_LOCK`, `X11_FOCUS_QUERY_LOCK`, `DISPLAY_ENV_LOCK`
  (integration_tests.rs:26–29) serialize tests that mutate global env vars
  (`WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `DISPLAY`). All five
  static `Mutex<()>` items keep `pub(super)` visibility so any test file in
  the same module tree can hold the lock. Verify after PR-1 that the
  lifecycle SNI tests still serialize. Acceptance: a `cargo test
  test_sni_runtime_managed_` filtered run still passes.

- **Proptest macro.** `proptest!{...}` is a procedural macro that expands
  to ordinary `mod` + `#[test]` items. It works identically inside a
  submodule. The two `proptest!` blocks (tests.rs:497, 1958) move with their
  surrounding test cluster (`dbus_naming.rs` and `focus_property.rs`
  respectively); strategy `fn`s stay co-located with their consumer.

- **`mod tests;` resolving to a directory.** Rust resolves `mod foo;` to
  `foo.rs` if it exists, else `foo/mod.rs`. When both exist, this is an
  error (rustc rejects it). We must delete `tests.rs` in the same commit
  that creates `tests/mod.rs`. Identical for `integration_tests.rs`. PR-1
  and PR-4 enforce this via `git mv` + Write in a single commit.

- **`cargo nextest` compatibility.** Constraint #10 says X11 tests use
  Xvfb on hardcoded displays `:100/:101/:102/:104` and nextest isolates each
  test in its own process. The split changes nothing about this. Note in
  the plan; do not touch.

- **Auto-generated `pub use` namespace pollution.** The `tests/mod.rs`
  `pub(super) use common::*;` re-export does *not* leak helpers into the
  crate API (they remain test-only via `#[cfg(test)] mod tests;`). Safe.

## 8. Risks and rollbacks

1. **Helper visibility omission.** Most likely failure mode. The fix is
   always local (add `pub(super)`). Compile errors surface immediately. Each
   PR is independently revertable.

2. **Test attribute drift during cut/paste.** Risk is highest for tests with
   `#[tokio::test(flavor = "multi_thread", worker_threads = N)]` (60 of the
   65 integration tests use this form). Mitigation: each PR's diff is
   reviewed for `#[test]`/`#[tokio::test]` line counts (`grep -cE
   "^#\[(tokio::)?test"` before and after). The before/after counts must
   match per file.

3. **Cross-module helper dependency.** PR-4's `start_wayland_test_server`
   needs `WaylandMockServer` from `wayland_mock` (Wayland-tests-local). The
   cleanest fix is to relocate `start_wayland_test_server` into `wayland.rs`
   itself in PR-5, leaving `WAYLAND_ENV_LOCK` in `common/polling.rs`.
   Documented above; will be confirmed at PR-4 build time.

4. **Misfiled-test-section discovery.** The "GNOME Extension State Parsing
   Tests" banner covers 4 different subsystems (gnome-ext-state, lifecycle
   restart/shutdown, logind decoding, focus_pipeline VK filtering). PR-2's
   inventory must categorize each test individually, not by banner. The
   plan does this in §2.

5. **`cargo test --bin kanata-switcher` count silently changes.** The
   `proptest!` macro generates one or more `#[test]` cases per `prop_*` fn.
   The total count of 261 includes 5 prop tests (one block of 1, one block
   of 5) plus 1 default proptest expansion adjustment. **Capture the exact
   `261 passed / 0 failed` line** as the pre-PR baseline in PR-1's
   description and assert it in every subsequent PR.

## 9. Out of scope / follow-ups

- Deduplicating `with_test_timeout` between `tests/common/` and
  `integration_tests/common/`. Would require a new top-level test-support
  module under `src/daemon/`. Not requested.
- Renaming the misnamed "GNOME Extension State Parsing Tests" banner in the
  original file. PR-3 simply distributes its contents to the correct files;
  the banner ceases to exist post-split.
- Fixing the misplaced `test_kde_focus_push_script_targets_per_instance_name`
  (a sync unit test sitting at the end of `integration_tests.rs:7146`). It
  could move to `tests/kde_script_paths.rs` instead of
  `integration_tests/dbus_multiplex.rs`. Cleaner, but it touches the
  `tests` vs. `integration_tests` boundary. Keep with multiplex in PR-5;
  flag for a later cleanup PR if the user prefers it under unit tests.
- Splitting `MockKanataServer` further (e.g., extracting `KanataMessage`
  into its own file). Single-purpose helpers; keep co-located.
- Investigating the two flaky X11 tests reportedly observed under parallel
  display contention. Plan acknowledges; does not fix.
- Adding `#[ignore]`-by-default attributes for tests that require
  Xvfb/dbus-daemon/dconf. They currently `panic!` with a helpful message if
  the binary is absent. Behavior preserved.
