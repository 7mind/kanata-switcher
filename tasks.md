# kanata-switcher — Task Ledger

Authoritative ledger of planned and completed work for the `main.rs` refactor.
Project conventions: `CLAUDE.md` and `llm-docs/`.

Status: `[ ]` planned · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Milestones (high-level)

- [x] **M0** — Produce a refactor plan for `src/daemon/main.rs` into a trait-based multi-file architecture.
- [x] **M1** — Execute PR-00..PR-15 from the plan, one PR at a time, behaviour-preserving. Completed 2026-05-12. main.rs: 7983 → 250 LOC.
- [~] **M2** — Split `tests.rs` (4864 LOC) and `integration_tests.rs` (7176 LOC) into per-backend/per-subsystem files.

---

## Milestone 2 — PR breakdown

Detail in `./docs/drafts/20260513-1040-test-split-plan.md`.

- [x] **M2-PR-01** — Convert `tests.rs` → `tests/` directory; extract `tests/common/helpers.rs`.
- [x] **M2-PR-02** — Split `tests/mod.rs` by section.
- [ ] **M2-PR-03** — Split Runtime Lifecycle into `tests/lifecycle/` subtree (fixtures, restart_or_shutdown, logind_decode, display_apply, runtime_target, persistent_dbus, sni_runtime, transition, provider, supervisor).
- [ ] **M2-PR-04** — Convert `integration_tests.rs` → `integration_tests/` directory; extract `integration_tests/common/` (polling, mock_kanata, focus_service, dbus_session).
- [ ] **M2-PR-05** — Split integration tests by backend (gnome/, kde/, wayland, x11, vk_validation, dbus_control, dbus_session_tests, dbus_multiplex, dconf).
- [ ] **M2-PR-06** — *(optional)* Further-split `dbus_session_tests.rs` if it exceeds the 1500-LOC ceiling.

---

## Milestone 0 — PR breakdown

Detail in `./docs/drafts/20260511-2128-mainrs-refactor-plan.md`.

- [x] **PR-PLAN** — Produce `./docs/drafts/20260511-2128-mainrs-refactor-plan.md` covering goals/non-goals, current-structure inventory, target module tree, trait seams, PR breakdown, cross-cutting concerns, risks, and follow-ups. Adversarially reviewed; iterate to clean.

---

## Milestone 1 — PR breakdown

Detail will live in `./docs/drafts/20260511-2128-mainrs-refactor-plan.md` (the plan IS the breakdown). Per-PR rows below are added when M1 is opened.

- [x] **PR-00** — Pre-flight: toolchain probe (`async fn` in trait + `wayland_scanner` macro path resolution).
- [x] **PR-01** — Extract `constants.rs`, `errors.rs`, `environ.rs` (plan-prose renamed from `env.rs`; see PR-01-D01).
- [x] **PR-02** — Extract `dbus_naming.rs` (+ `DbusSuffixError` into `errors.rs`).
- [x] **PR-03** — Extract `config.rs` and `focus.rs`.
- [x] **PR-04** — Extract `args.rs`, `autostart.rs`, `broadcasters.rs`, `kanata.rs` (split into 4a/4b/4c).
  - [x] **PR-04a** — `args.rs` + `autostart.rs`.
  - [x] **PR-04b** — `broadcasters.rs`.
  - [x] **PR-04c** — `kanata.rs` (+ `ShutdownGuard`).
- [x] **PR-05** — Extract `control/{mod,client}.rs`.
- [x] **PR-06** — Extract `pause.rs` and `focus_pipeline.rs`.
- [x] **PR-07** — Extract `lifecycle/` (mod, startup, logind, snapshot helpers).
- [x] **PR-08** — Extract `supervisor/` (mod, capabilities) + top-level `display_override.rs` (D01).
- [x] **PR-09** — Extract `backends/wayland/` including `wayland_scanner` protocol modules.
- [x] **PR-10** — Extract `backends/x11.rs` and `backends/linux_console.rs`.
- [x] **PR-11** — Extract `backends/gnome.rs` and `backends/kde/`.
- [x] **PR-12** — Introduce `FocusBackend` trait; retire `run_*_backend_task` adapters.
- [x] **PR-13** — Extract `control/server.rs` and `control/persistent.rs`.
- [x] **PR-14** — Extract `sni/` (single-PR; not split).
- [x] **PR-15** — Extract `gnome_ext/` (with `embed.rs` relative-path bump).

---

## Cross-cutting architectural notes (locked)

- [x] **`FocusBackend` uses `Pin<Box<dyn Future + Send + 'static>>`, not RPIT-in-traits** (PR-00 finding). RPIT-in-traits is not dyn-compatible on rustc 1.92.0; `Box<dyn FocusBackend>` dispatch in the supervisor requires the boxed-future shape. Each impl wraps its async block in `Box::pin(async move { ... })`.
- [x] **`wayland_scanner = 0.31.8` macros resolve XML paths via `CARGO_MANIFEST_DIR`** (PR-00 finding). PR-09 moves `mod cosmic_workspace` / `mod cosmic_toplevel` to `backends/wayland/protocols.rs` with the original path strings unchanged.
- [x] **V0 baseline is `cargo build && cargo test`, NOT the original plan's full clippy/fmt gate.** The codebase has ~28 pre-existing clippy errors and fmt drift on `main` and `persistent-daemon` that predate this refactor. Treating these as PR gates would conflate refactor regressions with unrelated tech debt. Each refactor PR is judged against "cargo build / cargo test pass" + "no NEW clippy/fmt regression vs the previous PR." A separate cleanup track can address the pre-existing debt; out of scope for this refactor.
- [x] **Bin entrypoint stays `src/daemon/main.rs`** — `Cargo.toml [[bin]].path` is not changed.
- [x] **Tests stay in place** — `src/daemon/tests.rs` and `src/daemon/integration_tests.rs` remain attached to `main.rs` as `#[cfg(test)] mod tests;` / `mod integration_tests;`; they continue to `use super::*;`.
- [x] **Test-mod access via a single `#[cfg(test)] pub(crate) use ...` re-export block in `main.rs`** — preserves `use super::*` semantics across the refactor.
- [x] **No new dependencies** — refactor is not the place to add crates.
- [x] **No behaviour changes** — identical CLI flags, DBus interface/path/signal/method names, log lines that tests grep for, config schema, on-disk file layout.
- [x] **Default visibility: `pub(crate)`** — keep existing `pub` on `KanataClient` and `Environment`.
- [x] **Trait policy** — introduce only `FocusBackend`. Reuse existing `DconfBackend`, `SniControlOps`. Reject `LifecycleProvider` / `KanataClient` / broadcaster traits as overengineering.
- [ ] **`wayland_scanner` macro path resolution** — PR-00 must verify `CARGO_MANIFEST_DIR`-relative resolution on `wayland-scanner = 0.31.8`; fallback is `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")`.
- [ ] **`gnome_ext_file!` macro relative-path bump in PR-15** — `../../src/gnome-extension/...` becomes `../../../src/gnome-extension/...` when the macro moves into `src/daemon/gnome_ext/embed.rs`.

---

## Completed

- **M2-PR-02** (2026-05-13) — Split `src/daemon/tests/mod.rs` (4768 LOC, 189 tests) into 10 leaf files by subsystem. Behaviour-preserving. Test attributes byte-identical.
  - **`focus_flow.rs`** (711 LOC): 32 tests — focus flow + virtual keys / fallthrough + paused status reset.
  - **`focus_pipeline.rs`** (193 LOC): 6 tests — `update_status_for_focus` (sync + async), including 2 misfiled tests rescued from the "GNOME Extension State Parsing Tests" banner.
  - **`focus_property.rs`** (279 LOC): 1 `proptest!` block (5 prop tests) + 8 strategy helpers.
  - **`autostart.rs`** (58 LOC): 3 autostart desktop-entry tests.
  - **`dbus_naming.rs`** (303 LOC): 28 dbus_naming/suffix/Args tests + 1 `proptest!` block.
  - **`control_commands.rs`** (31 LOC): 4 `resolve_control_command` tests.
  - **`kde_script_paths.rs`** (69 LOC): 4 KWin script-path tests + `build_kde_focus_push_script` test.
  - **`sni_presentation.rs`** (484 LOC): 17 SNI format/icon/state/settings/menu/tooltip/title tests (plan said 14; actual audit shows 17 legitimate SNI tests).
  - **`gnome_ext_state.rs`** (46 LOC): 3 actual gnome-extension state-parsing tests (the rest of the misnamed banner relocated to focus_pipeline.rs and stay in mod.rs awaiting M2-PR-03 lifecycle split).
  - **`config_parsing.rs`** (119 LOC): 8 config parsing tests.
  - **`tests/mod.rs`** (residual, 2499 LOC): 86 test attributes still here — 3 `wait_for_restart_or_shutdown_*` tests + 9 logind decode tests + the entire Runtime Lifecycle block (~73 tests). All move in M2-PR-03.
  - **No daemon-side widenings.** No production source files modified.
  - **Verification**: `cargo build` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **M2-PR-01** (2026-05-13) — Converted `src/daemon/tests.rs` (4864 LOC) from flat file to directory module `src/daemon/tests/`. Extracted 11 shared helpers (1 const, 1 static, 9 fns) to `tests/common/helpers.rs`. All 189 tests stay in `tests/mod.rs` for this PR — splitting them by section is M2-PR-02's job.
  - **`tests/common/helpers.rs`** (101 LOC, new): `TEST_TIMEOUT` const, `SNI_WATCHER_TEST_LOCK` static, `with_test_timeout`, `win`, `rule`, `rule_vk`, `rule_raw_vk`, `rule_with_fallthrough`, `has_action`, `get_layers`, `get_raw_vk_actions`.
  - **`tests/common/mod.rs`** (3 LOC, new): `use super::*; mod helpers; pub(super) use helpers::*;`.
  - **`tests/mod.rs`** (4768 LOC, new): all 189 tests + `mod common; pub(super) use common::*;` at the top. Old `tests.rs` deleted.
  - **`src/daemon/main.rs`**: unchanged. `#[cfg(test)] mod tests;` resolves to the new directory module automatically.
  - **Daemon-side check**: `KanataClient::inner` (`kanata.rs`), `pause::TEST_LAST_UNPAUSE_REQUEST_ENV`, `sni::indicator::ACTIVE_SNI_WATCHER_TASKS` all already `#[cfg(test)] pub(crate)` from M1. No daemon edits needed.
  - **Plan deviation (no defect)**: Helper visibility is `pub(crate)` rather than the planned `pub(super)`. The executor judged `pub(super)` insufficient for the helpers → common → tests re-export chain. Acceptable: the entire `tests` module is `#[cfg(test)]`-gated, so `pub(crate)` has no production surface impact. M2-PR-02 will likely need the same visibility for newly-extracted leaf files.
  - **Verification**: pre-move `grep -cE "^(#\[test\]|#\[tokio::test)" tests.rs` = 189; post-move on `tests/mod.rs` = 189 ✓. `cargo build` ✓ (26 baseline warnings, no new); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-15** (2026-05-12) — Extracted `src/daemon/gnome_ext/` (4 files). FINAL PR. Behaviour-preserving move; 722 LOC moved.
  - **`gnome_ext/embed.rs`** (58 LOC, new, `#[cfg(feature = "embed-gnome-extension")]`): the `gnome_ext_file!` macro with **the critical relative-path bump from `concat!("../../", ...)` to `concat!("../../../", ...)`** (one level deeper because the file moved from `src/daemon/main.rs` to `src/daemon/gnome_ext/embed.rs`). All 9 `EMBEDDED_*` `include_str!` consts. `compile_gnome_schemas`, `write_embedded_extension_to_dir`.
  - **`gnome_ext/detection.rs`** (290 LOC, new): `GnomeDetectionMethod`, `GnomeExtensionStatus`, `GnomeDbusProbeResult`, `gnome_state_name`, `parse_gnome_extension_state`, `is_dbus_service_unavailable`, `gnome_extension_dbus_probe`, `gnome_extension_dbus_probe_with_connection`, `gnome_extension_status`, `wait_for_session_bus_name_owner`, `session_bus_name_has_owner` (the PR-08 widening's final home).
  - **`gnome_ext/install.rs`** (150 LOC, new): `get_gnome_extension_fs_path`, `gnome_extension_fs_exists`, `pack_and_install_from_dir`, `install_gnome_extension`, `enable_gnome_extension`.
  - **`gnome_ext/mod.rs`** (224 LOC, new): submodule declarations + `pub(crate) use` re-exports, plus orchestration fns `print_gnome_extension_install_instructions`, `print_gnome_extension_status`, `ensure_gnome_extension`, `setup_gnome_extension`.
  - **`main.rs`**: 958 → 250 LOC. **96.9% reduction from the pre-refactor 7983 LOC.** Well below the §1 plan target of ≤400 LOC.
  - **`supervisor/capabilities.rs`**: two call sites updated `crate::session_bus_name_has_owner` → `crate::gnome_ext::detection::session_bus_name_has_owner`.
  - **Verification**: `cargo build` (default features = `embed-gnome-extension`) ✓ — confirms the relative-path bump worked; `cargo build --no-default-features` ✓ — confirms the `#[cfg]` gating is clean; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-14** (2026-05-12) — Extracted `src/daemon/sni/` directory module (8 files). Largest single-PR extraction yet (~1015 LOC moved). Behaviour-preserving.
  - **`sni/mod.rs`** (97 LOC, new): `SniControl`, `SniControlMode`, `SniRuntimeTransitionPlan`, `SniRuntimeWakeReason`, `sni_control_mode_for_environment`, `plan_sni_runtime_transition`, `wait_for_sni_runtime_wake_with_delay`. `pub(crate) use` re-exports of all 7 submodules.
  - **`sni/settings.rs`** (119 LOC, new): `DconfBackend` trait, `ShellDconfBackend`, `SniSettingsStore`, `dconf_get_bool`, `dconf_set_bool`, `is_dconf_unavailable`. `SniSettingsStore.available` widened to `pub(crate)` for integration test access.
  - **`sni/state.rs`** (73 LOC, new): `MenuRefresh`, `SniIndicatorState`.
  - **`sni/indicator.rs`** (428 LOC, new): `SniIndicator` + `impl Tray`, `start_sni_indicator`, `SniIndicatorRuntimeHandle` + Drop, `ACTIVE_SNI_WATCHER_TASKS` static + `SniWatcherTaskGuard` + `sni_watcher_task_count` (all `#[cfg(test)]`). Re-exports `ksni::{Icon as SniIcon, MenuItem, Status as SniStatus, ToolTip, Tray, TrayService}` for test access.
  - **`sni/control_local.rs`** (19 LOC, new): `SniLocalControl` struct (fields `pub(crate)` so `control_ops.rs` can pattern-match).
  - **`sni/control_dbus.rs`** (12 LOC, new): `SniDbusControl` struct (fields `pub(crate)` same reason).
  - **`sni/control_ops.rs`** (110 LOC, new): `SniControlOps` trait + `impl SniControlOps for SniControl` (dispatches via `match self { SniControl::Local(c) => c.field, SniControl::Dbus(c) => c.field }`).
  - **`sni/guard.rs`** (222 LOC, new): `SniGuard` + impl (`disabled`, `runtime_managed`, `runtime_managed_with_builder`), `build_sni_control_for_mode`, `SNI_RUNTIME_RETRY_INTERVAL` constant.
  - **`main.rs`**: 1979 → 958 LOC. Added `mod sni;` + `use sni::*;`. Removed `ksni::*` and `noto_sans_mono_bitmap::*` imports (now in submodules). Extended `#[cfg(test)] pub(crate) use crate::{...}` with `sni::*` plus all submodule globs per D24 discipline.
  - **Verification**: `cargo build` ✓ (31 warnings — accumulation; cleanup deferred); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed. SNI tests are numerous; green run confirms behaviour-preserving move.

- **PR-13** (2026-05-12) — Extracted `src/daemon/control/server.rs` and `src/daemon/control/persistent.rs` from `src/daemon/main.rs`. Behaviour-preserving move of the DBus control surface and persistent-reconnect manager.
  - **`control/server.rs`** (297 LOC, new): `DbusWindowFocusService` + its full `#[zbus::interface]` impl (WindowFocus, GetStatus, GetPaused, Pause, Unpause, Restart, signal emitters) kept colocated (zbus macro requires same file), `resolve_runtime_unpause_context`, `DbusServiceRegistration` + Drop, `register_dbus_service` (cfg-test), `register_dbus_service_with_runtime_environment`.
  - **`control/persistent.rs`** (248 LOC, new): `DBUS_RECONNECT_DELAYS_MS` constant (per PR-01 D07), `dbus_reconnect_delay`, `wait_for_dbus_reconnect_retry`, `PersistentDbusServiceGuard` + Drop, `start_persistent_dbus_service`, `start_persistent_dbus_service_with_connector` (generic), `run_persistent_dbus_service_with_connector` (generic).
  - **`control/mod.rs`**: added `pub(crate) mod server; pub(crate) mod persistent;` and `pub(crate) use {server::*, persistent::*};` so test re-exports through `control::*` reach submodule items.
  - **`main.rs`**: 2501 → 1979 LOC. No supervisor or other call-site updates needed — all consumers were in main.rs itself.
  - **Verification**: `cargo build` ✓ (25 warnings — minor uptick from unused imports; cleanup deferred); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed. DBus interface tests pass — confirms zbus macro relocation was clean.

- **PR-12** (2026-05-12) — Introduced the `FocusBackend` trait and replaced supervisor's 5-way `RuntimeTarget` match arm with trait dispatch. First semantic-shaped change; behaviour identical (same backends, same calls, dispatched via `Box<dyn FocusBackend>`). PR-00 finding (boxed-future shape, not RPIT-in-traits) honoured throughout.
  - **`backends/mod.rs`** (+38 LOC, now 129 LOC total): moved `BackendExit` and `map_run_outcome_to_backend_exit` from `main.rs` (they had been widened to `pub(crate)` since PR-08; widening reverted because they no longer live in main.rs). Added `pub(crate) struct BackendRunContext { kanata, focus_handler, status_broadcaster, pause_broadcaster, restart_handle, shutdown_handle, effective_bus_name, display_override }` — note the plan's `environment` field was dropped (no impl reads it; the per-backend `run_xxx` knows its environment implicitly). Added `pub(crate) trait FocusBackend: Send + 'static { fn run(self: Box<Self>, ctx: BackendRunContext) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>>; }`.
  - **Per-backend impls** (one struct + one `impl FocusBackend` per file): `GnomeBackend` in `backends/gnome.rs`, `KdeBackend` in `backends/kde/mod.rs`, `WaylandBackend` in `backends/wayland/mod.rs`, `X11Backend` in `backends/x11.rs`, `LinuxConsoleBackend` in `backends/linux_console.rs`. Each `run` body just `Box::pin`-s an `async move` that unpacks `BackendRunContext`, awaits the existing free `run_xxx`, and maps the `RunOutcome` via `map_run_outcome_to_backend_exit`. `LinuxConsoleBackend` got its body inlined from the former supervisor adapter (the only one with substantive body — the other four delegate to existing free functions).
  - **`supervisor/mod.rs`**: replaced the 5-arm `RuntimeTarget` match in `start_backend` with `Box::new(XxxBackend) as Box<dyn FocusBackend>` per arm. Deleted all five `run_*_backend_task` adapter functions (net -188 LOC).
  - **`BackendContext`** (supervisor-level state) was kept distinct from `BackendRunContext` (backends-layer arg-bundle) — the supervisor context includes GNOME-setup fields and `runtime_environment` that backends don't need. This keeps supervisor concerns out of the backends layer.
  - **`main.rs`**: 2514 → 2501 LOC (just the `BackendExit` + helper moved out — 13 LOC).
  - **Verification**: `cargo build` ✓ (21 warnings — unchanged from PR-11; no new ones); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed. No `Send`-propagation issues; all impls capture only `Send` types (KanataClient, Arc<Mutex<...>>, channel broadcasters).

- **PR-11** (2026-05-12) — Extracted `src/daemon/backends/gnome.rs` and `src/daemon/backends/kde/{mod,script,probe}.rs`. Plus moved the dispatch helpers `query_focus_for_env` and `apply_focus_for_env` into `backends/mod.rs`. ~800 LOC moved across 4 files.
  - **`backends/gnome.rs`** (145 LOC, new): `query_gnome_focus`, `run_gnome`, `GnomeFocusSignalSubscription` + `Drop`, `subscribe_to_gnome_focus_signal`.
  - **`backends/kde/mod.rs`** (213 LOC, new): `pub(crate) mod probe; pub(crate) mod script; pub(crate) use {probe::*, script::*};` (per D24), `KwinScriptGuard` + `Drop`, `run_kde`.
  - **`backends/kde/script.rs`** (170 LOC, new): `KDE_QUERY_COUNTER` static, `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, `KdeFocusQueryService` + zbus interface impl, `kwin_script_object_path`, `load_kwin_script`, `build_kde_query_script`, `build_kde_focus_push_script`.
  - **`backends/kde/probe.rs`** (247 LOC, new): `resolve_kde_runtime_query_mode_with_retry`, `ensure_kde_scripting_ready`, `resolve_kde_runtime_query_mode`, `kwin_object_path_exists`, `unload_kwin_script_by_path`, `remove_kwin_probe_script_file`, `environment_requires_focus_query_connection`, `query_kde_focus`.
  - **`backends/mod.rs`** (+70 LOC): added `pub(crate) mod gnome; pub(crate) mod kde;` and `pub(crate) use {gnome::*, kde::*};`. Also moved in `query_focus_for_env` and `apply_focus_for_env` dispatch helpers (these were widened to `pub(crate)` in PR-06; revert at this PR since they now live in their natural home and consumers can use `crate::backends::*` paths).
  - **`main.rs`**: 3300 → 2514 LOC. Extended `#[cfg(test)] pub(crate) use crate::{...}` with `backends::gnome::*, backends::kde::*, backends::kde::script::*, backends::kde::probe::*`.
  - **`supervisor/mod.rs`**: updated `crate::{run_gnome, run_kde}` imports to `crate::backends::gnome::run_gnome` and `crate::backends::kde::run_kde`.
  - **`pause.rs`**: updated `use crate::apply_focus_for_env` → `use crate::backends::apply_focus_for_env`.
  - **Verification**: `cargo build` ✓ (21 warnings — accumulation continues; deferred cleanup); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-10** (2026-05-12) — Extracted `src/daemon/backends/x11.rs` from `main.rs`; created placeholder `backends/linux_console.rs`. Behaviour-preserving move.
  - **`backends/x11.rs`** (233 LOC, new): `x11rb::atom_manager!` block (per D11), `X11State` + impl, `run_x11`, `query_x11_active_window`. All `pub(crate)`.
  - **`backends/linux_console.rs`** (5 LOC, placeholder): comment-only. `run_linux_console_backend_task` stays in `supervisor/mod.rs` — moving it would create a backends → supervisor cycle because it takes `BackendContext` (a supervisor type). PR-12 will absorb it into a `FocusBackend` impl alongside the other backend_task adapters.
  - **`backends/mod.rs`**: added `pub(crate) mod x11; pub(crate) mod linux_console;` and `pub(crate) use x11::*;`.
  - **`main.rs`**: 3526 → 3300 LOC. Removed atom_manager block + X11State + impl + run_x11 + query_x11_active_window + orphaned x11rb/AsyncFd/AsRawFd imports.
  - **`supervisor/mod.rs`**: updated `crate::run_x11` import line to `use crate::backends::x11::run_x11;`.
  - **Verification**: `cargo build` ✓ (17 warnings — minor accumulation continues; deferred cleanup); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-09** (2026-05-12) — Extracted `src/daemon/backends/` and `src/daemon/backends/wayland/` directory modules. Highest-macro-risk PR (wayland_scanner generators in a nested module).
  - **`backends/mod.rs`** (22 LOC, new): `RawFdWatcher` (shared by wayland and x11 per D05); `pub(crate) mod wayland; pub(crate) use wayland::*;`.
  - **`backends/wayland/mod.rs`** (271 LOC, new): `ToplevelWindow`, `WaylandState` + inherent impl, `WaylandProtocol`, `run_wayland`, `resolve_wayland_socket_path`, `connect_wayland_with_display_override`, `query_wayland_active_window`, `wayland_query_count` + `WAYLAND_QUERY_COUNTER` static. `pub(crate) mod protocols; pub(crate) mod dispatch_common; pub(crate) mod dispatch_wlr; pub(crate) mod dispatch_cosmic; pub(crate) use {protocols::*, dispatch_common::*, dispatch_wlr::*, dispatch_cosmic::*};` (per D24).
  - **`backends/wayland/protocols.rs`** (30 LOC, new): `pub(crate) mod cosmic_workspace { wayland_scanner::generate_interfaces!(...); generate_client_code!(...); }` and `pub(crate) mod cosmic_toplevel { ... }`. Per D03, intra-module paths fixed: `crate::cosmic_workspace::__interfaces::*` → `super::super::cosmic_workspace::__interfaces::*` and `crate::cosmic_workspace::*` → `super::cosmic_workspace::*`.
  - **`backends/wayland/dispatch_common.rs`** (31 LOC, new): `wl_registry::WlRegistry` + `wl_output::WlOutput` Dispatch impls.
  - **`backends/wayland/dispatch_wlr.rs`** (69 LOC, new): `ZwlrForeignToplevelManagerV1` + `ZwlrForeignToplevelHandleV1` Dispatch impls.
  - **`backends/wayland/dispatch_cosmic.rs`** (124 LOC, new): all 5 cosmic Dispatch impls.
  - **`main.rs`**: 4026 → 3526 LOC. Added `mod backends;` + `use backends::*; use backends::wayland::*;`, extended `#[cfg(test)] pub(crate) use crate::{...}` with `backends::*, backends::wayland::*`. Removed top-level `mod cosmic_workspace;` / `mod cosmic_toplevel;` and the orphaned `use cosmic_*::{...}` lines.
  - **`supervisor/mod.rs`**: updated `crate::run_wayland` to `crate::backends::wayland::run_wayland`. Other widened items (`run_gnome`, `run_kde`, `run_x11`) still come from crate root until PR-10/PR-11.
  - **Other fixes mid-extraction**: `ShutdownHandle` import path corrected from `crate::pause::ShutdownHandle` to `crate::ShutdownHandle`; added `Proxy` trait import to `dispatch_wlr.rs` and `dispatch_cosmic.rs` for `.id()` resolution; `ToplevelWindow` and `WaylandState` widened to `pub(crate)` for cross-module access.
  - **Verification**: `cargo build` ✓ (8 → 18 warnings; all unused-import accumulation from PR-07/08/09; clippy audit on dispatch_cosmic.rs per D14 found no real lint hits); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-08** (2026-05-12) — Extracted top-level `src/daemon/display_override.rs` (per D01) and `src/daemon/supervisor/{mod,capabilities}.rs` from `src/daemon/main.rs`. Largest single-PR extraction yet (~920 LOC moved).
  - **`display_override.rs`** (190 LOC, new, top-level — NOT under `supervisor/` per D01): `display_override_expected_session_type`, `is_valid_wayland_display_override`, `normalize_display_override`, `resolve_display_override_from_logind`, `display_override_backend_kind_for_environment`, `resolve_display_override_for_backend_kind`, `resolve_display_override_for_environment`, plus test-only `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE`, `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE`, `TestFocusQueryDisplayOverrideGuard`, `display_override_test_slot`, `set/resolve_test_focus_query_display_override` (cfg-test + cfg-not(test) variants). Top-level placement breaks the backends → supervisor cycle that `query_focus_for_env` would otherwise create in PR-11.
  - **`supervisor/capabilities.rs`** (34 LOC, new): `detect_desktop_capabilities`, `resolve_runtime_target_for_snapshot`.
  - **`supervisor/mod.rs`** (696 LOC, new): `WAYLAND_CAPABILITY_RECHECK_INTERVAL` (PR-01 D07), `BackendContext`, `BackendHandle`, runtime_target helpers, the five `run_*_backend_task` adapters, `ensure_runtime_gnome_extension_setup`, `start_backend`, `SupervisorState`, `transition_runtime_target[_with_starter]`, `stop_current_backend`, `run_lifecycle_supervisor[_with_starter][_with_starter_and_resolver]`, `wait_for_wayland_capability_recheck`, `wait_for_backend_completion_signal`, `poll_finished_backend_outcome`. `pub(crate) use capabilities::*;` at the top so test re-exports reach submodule items (D13).
  - **`main.rs`**: 4911 → 4026 LOC. Added `mod display_override; mod supervisor;` + `use display_override::*; use supervisor::*;`. Extended `#[cfg(test)] pub(crate) use crate::{...}` with `display_override::*, supervisor::*, supervisor::capabilities::*`.
  - **Visibility widenings** (per D06 + extras): `run_gnome`, `run_kde`, `run_wayland`, `run_x11`, `query_gnome_focus`, `query_kde_focus`, `query_wayland_active_window`, `query_x11_active_window`, `BackendExit`, `map_run_outcome_to_backend_exit` all widened to `pub(crate)` (these all stay in main.rs until PR-09/PR-10/PR-11/PR-12). Plus the unplanned `session_bus_name_has_owner` widening — needed by `capabilities.rs`; reverts when that helper moves to `gnome_ext/detection.rs` in PR-15.
  - **Verification**: `cargo build` ✓ (8 warnings — 1 baseline + 7 from PR-07's incomplete unused-import cleanup; no new warnings from this PR); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.
  - **Notes**: Executor caught their own bug mid-extraction — initially deleted `BackendExit` and `map_run_outcome_to_backend_exit` along with the supervisor block; restored them since they stay in main.rs until PR-12 (per D23).

- **PR-07** (2026-05-12) — Extracted `src/daemon/lifecycle/` directory module (mod.rs + logind.rs + startup.rs + snapshot.rs). Behaviour-preserving move of the entire lifecycle-provider cluster.
  - **`lifecycle/mod.rs`** (55 LOC, new): `pub(crate) mod {logind, startup, snapshot};` + `pub(crate) use {logind::*, startup::*, snapshot::*};` (so the bare `lifecycle::*` re-export in main.rs reaches submodule items per D13). `enum LifecycleProvider` + `impl`.
  - **`lifecycle/logind.rs`** (612 LOC, new): all logind parse/decode helpers (`resolve_logind_session_path` through `wait_for_logind_display_session_path`), `LogindSessionPathResolutionError`, `LogindDisplayPathChange`, `LogindDisplayChangeAction`, `LogindLifecycleProvider` + impl, monitor task helpers, `verify_logind_lifecycle_monitor_prerequisites`, `validate_active_logind_session_type`, `fail_fast_lifecycle_monitor`, `expect_some_or_fail_fast`, `expect_or_fail_fast`, `monitor_logind_lifecycle`, `open_logind_session_monitor`, `decode_logind_lifecycle_snapshot_change`.
  - **`lifecycle/startup.rs`** (18 LOC, new): `StartupSnapshotProvider` + impl.
  - **`lifecycle/snapshot.rs`** (9 LOC, new): `snapshot_no_session`.
  - **`main.rs`**: 5579 → 4911 LOC. Added `mod lifecycle;` + `use lifecycle::*; use lifecycle::logind::*; use lifecycle::startup::*;`. Extended `#[cfg(test)] pub(crate) use crate::{...}` with `lifecycle::*, lifecycle::logind::*, lifecycle::startup::*`.
  - **Visibility widenings**: `resolve_logind_session_path` and two helpers widened to `pub(crate)` because `resolve_display_override_from_logind` (still in main.rs until PR-08) calls `resolve_logind_session_path` directly.
  - **Verification**: `cargo build` ✓ (warning count went 1 → 7, all unused-import warnings from incomplete cleanup in main.rs — follow-up tracked); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-06** (2026-05-12) — Extracted `src/daemon/pause.rs` and `src/daemon/focus_pipeline.rs` from `src/daemon/main.rs`. Behaviour-preserving move.
  - **`pause.rs`** (131 LOC, new): `UnpauseContext`, `local_sni_unpause_context`, `pause_daemon`, `unpause_daemon`, plus the test-only `TEST_LAST_UNPAUSE_REQUEST_ENV` static + `record_unpause_request_environment_for_test` + `take_unpause_request_environment_for_test`.
  - **`focus_pipeline.rs`** (103 LOC, new): `execute_focus_actions`, `extract_focus_layer`, `update_status_for_focus`, `handle_focus_event`, `native_terminal_window`, `resolve_sni_focus_only`.
  - **`main.rs`**: 5791 → 5579 LOC. Added `mod pause; mod focus_pipeline;` + `use pause::*; use focus_pipeline::*;` (the glob from `pause::*` covers SNI + DBus server consumers of `UnpauseContext` still in main.rs per defect D21). Extended `#[cfg(test)] pub(crate) use crate::{...}` with `pause::*, focus_pipeline::*`.
  - **Visibility widenings in main.rs** (plan D19 + extras the executor needed): `query_focus_for_env`, `apply_focus_for_env`, `SniSettingsStore`, `SNI_DEFAULT_SHOW_FOCUS_ONLY` widened to `pub(crate)`. The first two will be reverted when they move into `backends/mod.rs` in PR-11; the SNI items move in PR-14.
  - **Verification**: `cargo build` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-05** (2026-05-12) — Extracted `src/daemon/control/` (first nested-directory module). Behaviour-preserving move.
  - **`control/mod.rs`** (33 LOC, new): `enum ControlCommand` + `impl`, `enum ControlDispatch`, `pub(crate) mod client;`. All `pub(crate)`.
  - **`control/client.rs`** (130 LOC, new): `BROADCAST_PER_CALL_TIMEOUT` constant (PR-01 D07 — used only by broadcast dispatch), `send_control_command`, `send_control_command_with_connection`, `BroadcastEntryReport`, `BroadcastReport`, `enumerate_daemon_names`, `send_control_command_broadcast`.
  - **`main.rs`**: 5945 → 5791 LOC. Added `mod control;` + `use control::{ControlCommand, ControlDispatch}; use control::client::*;`. Extended `#[cfg(test)] pub(crate) use crate::{...}` with `control::*, control::client::*`.
  - **`args.rs`**: changed `use super::ControlCommand;` to `use crate::control::ControlCommand;` (now that `ControlCommand` has moved out of main.rs).
  - **Verification**: `cargo build` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-04c** (2026-05-12) — Extracted `src/daemon/kanata.rs` from `src/daemon/main.rs`. Behaviour-preserving move of the daemon's kanata TCP client. Highest-coupling extraction so far — every backend consumes `KanataClient`.
  - **`kanata.rs`** (650 LOC, new): all kanata wire types (`*Msg`/`*Payload` for ChangeLayer, LayerChange, RequestLayerNames, ActOnFakeKey, LayerNames, RequestFakeKeyNames, FakeKeyNames), `KanataClientInner`, `pub struct KanataClient` (kept `pub` per plan §1 lock), the full `impl KanataClient` block, `struct ShutdownGuard` + `Drop` impl. Imports: `std::sync::Arc`, `std::time::Duration`, `tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader}`, `tokio::net::{TcpStream as TokioTcpStream, tcp::OwnedWriteHalf}`, `tokio::sync::Mutex as TokioMutex`, `serde::{Deserialize, Serialize}`, `crate::broadcasters::{LayerSource, StatusBroadcaster}`.
  - **`main.rs`**: 6588 → 5945 LOC. Added `mod kanata;` + `use kanata::*;`, extended `#[cfg(test)] pub(crate) use crate::{...}` with `kanata::*`. Removed 4 import lines orphaned by the move (`serde::{Deserialize, Serialize}`, `tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader}`, `tokio::net::TcpStream`, `tokio::net::tcp::OwnedWriteHalf`).
  - **Visibility widenings** (within `kanata.rs` itself, to support whitebox tests via `use super::*`): `KanataClient::new` → `pub(crate)`, `KanataClient::resolve_layer_name` → `pub(crate)`, `KanataClient::filter_valid_virtual_keys` → `pub(crate)`, `KanataClient::inner` field → `pub(crate)`, all `KanataClientInner` fields → `pub(crate)`.
  - **Verification**: `cargo build` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed. Integration tests with mock kanata TCP server pass — confirms the move was behaviour-preserving for the full client lifecycle (connect, reconnect-with-queue, layer-name resolution, VK fake-key API).

- **PR-04b** (2026-05-12) — Extracted `src/daemon/broadcasters.rs` from `src/daemon/main.rs`. Behaviour-preserving move.
  - **`broadcasters.rs`** (209 LOC, new): `StatusSnapshot`, `LayerSource`, `StatusBroadcaster`, `RestartHandle`, `PauseBroadcaster`, `RuntimeEnvironmentBroadcaster`, `ShutdownHandle`, `wait_for_restart_or_shutdown`. All `pub(crate)`.
  - **`main.rs`**: 6788 → 6588 LOC. Added `mod broadcasters;` + `use broadcasters::*;`, extended `#[cfg(test)] pub(crate) use crate::{...}` with `broadcasters::*`.
  - **Verification**: `cargo build` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.

- **PR-04a** (2026-05-11) — Extracted `src/daemon/args.rs` and `src/daemon/autostart.rs` from `src/daemon/main.rs`. Behaviour-preserving move.
  - **`args.rs`** (130 LOC, new): `enum TrayFocusOnly` + `impl`, `struct Args` (clap derive), `parse_dbus_suffix_arg`, `resolve_install_gnome_extension`, `resolve_control_command`. Imports: `clap::{ArgMatches, Parser, ValueEnum}`, `std::path::PathBuf`, `crate::dbus_naming::sanitize_dbus_suffix`, `super::ControlCommand` (still in main.rs at this PR — moves out in PR-05).
  - **`autostart.rs`** (182 LOC, new): the 8 autostart functions plus the 3 `AUTOSTART_*` constants (PR-01 D07 — used only by autostart code). Imports: `clap::ArgMatches`, `std::env`, `std::path::{Path, PathBuf}`, `crate::args::Args`, `crate::errors::DynError`.
  - **`main.rs`**: 7092 → 6788 LOC. Added `mod args; mod autostart;` + `use args::*; use autostart::*;`, extended `#[cfg(test)] pub(crate) use crate::{...}` with `args::*, autostart::*`.
  - **Verification**: `cargo build --bin kanata-switcher` ✓ (1 pre-existing warning); `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.
  - **Notes**: Subagent worked in a git worktree per the loop's parallel-edit discipline; orchestrator copied results back. `use super::ControlCommand` (rather than `crate::ControlCommand`) avoids a temporary crate-root re-export — `ControlCommand` lives in `main.rs` (the bin's crate root) so child modules can reach it via `super`. Acceptable; will be replaced when PR-05 moves `ControlCommand` to `control/mod.rs`.

- **PR-03** (2026-05-11) — Extracted `src/daemon/config.rs` and `src/daemon/focus.rs` from `src/daemon/main.rs`. Behaviour-preserving move.
  - **`config.rs`** (227 LOC, new): `struct Rule`, `struct NativeTerminalRule`, `enum ConfigEntry` (with custom `Deserialize` impl), `struct Config`, `struct WindowInfo`, `fn load_config`, `fn match_pattern`. All `pub(crate)`. Imports: `regex::Regex`, `serde::Deserialize`, `std::env`, `std::fs`, `std::path::{Path, PathBuf}`, `dirs` (unqualified).
  - **`focus.rs`** (331 LOC, new): `enum FocusAction`, `struct FocusActions`, `const NATIVE_TERMINAL_RULE_INDEX`, `struct FocusHandler` + full inherent impl block. Imports: `use crate::config::{NativeTerminalRule, Rule, WindowInfo, match_pattern};`.
  - **`main.rs`**: 7642 → 7092 LOC. Added `mod config; mod focus;` + `use config::*; use focus::*;`, extended the `#[cfg(test)] pub(crate) use crate::{...}` re-export glob with `config::*, focus::*`. Removed now-unused `use regex::Regex;`. No visibility widenings of items still in main.rs were needed (FocusHandler does not reference KanataClient or broadcasters directly).
  - **Verification**: `cargo build --bin kanata-switcher` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed (proptest invariants on FocusHandler pass).
  - **Notes**:
    - Plan §3 / D07 placed `NATIVE_TERMINAL_RULE_INDEX` in `config.rs`; executor moved it to `focus.rs` where its only consumer (`FocusHandler::handle`) lives. Plan amended to reflect actual placement.
    - Subagent committed PR-03 itself rather than returning for orchestrator commit. Acceptable since the result is clean and the commit message is on-topic; future executors should ideally leave commits to the orchestrator so the ledger gets the commit metadata in sync.

- **PR-02** (2026-05-11) — Extracted `src/daemon/dbus_naming.rs` from `src/daemon/main.rs` and appended `DbusSuffixError` to `src/daemon/errors.rs`. Behaviour-preserving move.
  - **`errors.rs`** (+23 LOC, now 24 LOC total): added `enum DbusSuffixError` + `impl Display` + `impl Error`.
  - **`dbus_naming.rs`** (83 LOC, new): `sanitize_dbus_suffix`, `derive_default_dbus_suffix`, `resolve_dbus_suffix`, `effective_dbus_name`, `is_daemon_bus_name`. All `pub(crate)`.
  - **`main.rs`**: 7743 → 7642 LOC. Added `mod dbus_naming;` + `use dbus_naming::*;`, extended the `#[cfg(test)] pub(crate) use crate::{...}` re-export glob with `dbus_naming::*`.
  - **Verification**: `cargo build --bin kanata-switcher` ✓; `cargo test --bin kanata-switcher -- --test-threads=4` → 261 passed / 0 failed.
  - **Notes**: Mechanical PR. No adversarial-review subagent dispatched — sanity-check (grep for left-behind references + build + tests pass) is sufficient for a 105-LOC self-contained move with no visibility surprises. Full review reserved for higher-risk PRs (PR-03 focus engine, PR-08 supervisor, PR-09 wayland, PR-11 backend split, PR-12 trait, PR-14 SNI).

- **PR-01** (2026-05-11) — Extracted `src/daemon/{constants,errors,environ}.rs` from `src/daemon/main.rs`. Behaviour-preserving move.
  - **`constants.rs`** (53 LOC): the opening 91–125 constant block + `GNOME_EXTENSION_SRC_PATH`/`GNOME_EXTENSION_SCHEMA_FILE`/`GNOME_EXTENSION_SCHEMA_COMPILED` + `DCONF_FOCUS_ONLY_KEY` + `GNOME_SHELL_BUS_NAME`/`GNOME_SHELL_OBJECT_PATH`/`GNOME_SHELL_EXTENSIONS_INTERFACE`/`DBUS_ERROR_SERVICE_UNKNOWN`/`DBUS_ERROR_NAME_HAS_NO_OWNER`/`DBUS_ERROR_UNKNOWN_METHOD`.
  - **`errors.rs`** (1 LOC): `pub(crate) type DynError = Box<dyn std::error::Error + Send + Sync>`.
  - **`environ.rs`** (202 LOC): `enum Environment` (kept `pub`) + `RunOutcome` + `SessionKind` + `DesktopFlavor` + `BackendKind` + `RuntimeTarget` + `DesktopCapabilities` + `LifecycleSnapshot` + `impl Environment` + 8 free functions.
  - **`main.rs`**: 7983 → 7743 LOC. Added `mod constants; mod errors; mod environ;` + `use constants::*; use errors::DynError; use environ::*;` + `#[cfg(test)] pub(crate) use crate::{constants::*, errors::*, environ::*};` re-export block for test-mod access.
  - **Verification**: `cargo build --bin kanata-switcher` ✓; `cargo test --bin kanata-switcher` → 261 passed / 0 failed.
  - **Notes / surprises**:
    - Plan called the module `env.rs`; executor renamed to `environ.rs` to avoid colliding with `use std::env;` (5 call sites in main.rs use `env::var*`). Accepted the rename via PR-01-D01; plan prose updated to use `environ` consistently.
    - Reserved-for-later constants (SNI_*, AUTOSTART_*, BROADCAST_PER_CALL_TIMEOUT, NATIVE_TERMINAL_RULE_INDEX, WAYLAND_CAPABILITY_RECHECK_INTERVAL, DBUS_RECONNECT_DELAYS_MS, SNI_RUNTIME_RETRY_INTERVAL) remain in `main.rs` and move with their respective clusters in later PRs.

- **PR-00** (2026-05-11) — Pre-flight probes. Two compile-only smoke tests under a temporary `src/daemon/_pr00_probe.rs` (module added then removed atomically in this PR; no production code change).
  - **Probe A — `wayland_scanner` path resolution from a nested module**: compiled `wayland_scanner::generate_interfaces!("src/protocols/cosmic-workspace-unstable-v1.xml")` and `generate_client_code!(...)` inside `pub mod probe_cosmic_workspace { pub mod __interfaces { ... } ... }`. **Outcome: COMPILES.** `wayland-scanner 0.31.8` resolves XML paths via `CARGO_MANIFEST_DIR`. PR-09 proceeds with the original path strings unchanged.
  - **Probe B — `FocusBackend` trait shape**: compiled the trait in two candidate forms with three checks each (trait decl, `Box<dyn FocusBackend>` dispatch, `tokio::spawn` of the returned future).
    - Candidate 1: RPIT-in-traits (`fn run(...) -> impl Future<...> + Send`). **REJECTED.** Trait declares fine, but `Box<dyn FocusBackend>` fails with E0038 ("not dyn compatible because method `run` references an `impl Trait` type in its return type") on rustc 1.92.0.
    - Candidate 2: boxed-future (`fn run(...) -> Pin<Box<dyn Future<Output = ...> + Send + 'static>>`). **ACCEPTED.** All three checks pass.
    - Decision: plan §4.1 and PR-12 step 2 now use the boxed-future shape; each impl wraps its async block in `Box::pin(async move { ... })`.
  - **Plan amendment**: `docs/drafts/20260511-2128-mainrs-refactor-plan.md` §4.1 (trait shape + impl skeleton), PR-00 scope (replaced toolchain-version probe + scaffold round-trip with the two actually-executed probes and their decisions), PR-12 step 2 (impl shape).
  - **Verification**: `cargo build --bin kanata-switcher` ✓; `cargo test --bin kanata-switcher --no-run` ✓ (compile-only — PR-00 makes no code change that would shift test behavior). Pre-existing baseline `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` failures (~28 errors) are unrelated to this refactor — see Cross-cutting note above for the V0 reality.
  - **Notes / surprises**:
    - The planning subagent assumed RPIT-in-traits would be dyn-compatible on a modern rustc. It is not (rustc 1.92.0 still emits E0038). The plan's `Box<dyn FocusBackend>` dispatch + RPIT-in-traits combo was internally inconsistent; the boxed-future shape was already listed as a documented fallback, so the fix was a single-edit decision rather than a re-architecture.
    - V0 in the plan over-specified the gate (`cargo clippy --all-targets -- -D warnings && cargo fmt --check`). The codebase did not pass these gates pre-refactor, so a stricter V0 would conflate refactor regressions with unrelated debt. Locked V0 as `cargo build && cargo test` in this session.
    - User moved the work from `persistent-daemon` to a new `trait-refactor` branch between turns (both branches point to the same commits 2a973f1 and b81adbb at PR-00 start). Subsequent PRs continue on `trait-refactor`.

- **PR-PLAN** (2026-05-11) — Produced `./docs/drafts/20260511-2128-mainrs-refactor-plan.md`: a 15-PR (PR-00 → PR-15) executable refactor plan for `src/daemon/main.rs` (7983 LOC → target ≤400 LOC). Module tree under `src/daemon/` with top-level files (`constants`, `errors`, `env`, `args`, `dbus_naming`, `autostart`, `config`, `focus`, `broadcasters`, `kanata`, `pause`, `focus_pipeline`, `display_override`) and `control/`, `lifecycle/`, `supervisor/`, `backends/{gnome,kde,wayland,x11,linux_console}`, `sni/`, `gnome_ext/` subtrees. Introduces one new trait — `FocusBackend` — with stable RPIT-in-traits `fn run(self: Box<Self>, ctx) -> impl Future<...> + Send`. Keeps existing `DconfBackend` and `SniControlOps`. Rejects `LifecycleProvider`/`KanataClient`/broadcaster traits as overengineering. Verification baseline V0 = `cargo build && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check && nix build && nix run .#test`.

  Adversarial review: 4 rounds, 25 defects raised and resolved across `./defects.md` (rounds 1/2/3/4 → 17/5/3/0 new defects; round-4 reviewer verdict: "PLAN IS CLEAN. Inner loop can close."). The major issues that surfaced and were corrected:
  - cycle: `query_focus_for_env` calls `resolve_display_override_for_environment`, forcing the display-override block out of `supervisor/` and into a top-level `display_override.rs` (D01);
  - missing test static: `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` was overlooked next to its X11 sibling (D02);
  - intra-module path: `crate::cosmic_workspace::*` references inside `mod cosmic_toplevel` need rewriting to `super::cosmic_workspace::*` after relocation (D03);
  - trait shape: `#[async_trait]` example violated the no-new-deps rule and stable AFIT does not auto-propagate `Send`; pinned the trait to RPIT-in-traits with explicit `+ Send` (D04, D22);
  - shared utility: `RawFdWatcher` is consumed by both Wayland and X11 — lives in `backends/mod.rs` (D05);
  - visibility transitions: PR-08 must temporarily widen `run_*`, `query_*_focus`, `query_focus_for_env`, `apply_focus_for_env`, `BackendExit`, `map_run_outcome_to_backend_exit` to `pub(crate)` for the duration of the supervisor extraction (D06, D19, D23);
  - constant inventory: 15 additional `const`s scattered across main.rs were missed in PR-01's initial enumeration (D07);
  - trait return type: `BackendExit` colocates with `FocusBackend` in `backends/mod.rs`, with `supervisor` importing it (D20);
  - submodule re-exports: `backends/kde/mod.rs` and `backends/wayland/mod.rs` must `pub(crate) use` their submodules so test-mod globs reach inner items (D24);
  - `gnome_ext_file!` macro relative-path bump: `../../src/gnome-extension/...` → `../../../src/gnome-extension/...` when the macro moves one level deeper (PR-15 risk).

  Verification of plan-as-document: `wc -l docs/drafts/20260511-2128-mainrs-refactor-plan.md` → ~520 lines. No source code edited this session.

  Constraints future work must respect:
  - PR-00 is gating: must confirm `wayland_scanner 0.31.8` resolves XML paths via `CARGO_MANIFEST_DIR` (else amend PR-09 upfront to `concat!(env!("CARGO_MANIFEST_DIR"), …)`), and must compile a one-line `impl FocusBackend for Stub { fn run(self: Box<Self>, _ctx: BackendRunContext) -> impl Future<Output = …> + Send { async move { … } } }` to confirm `+ Send` propagation on the installed rustc.
  - `src/daemon/tests.rs` and `src/daemon/integration_tests.rs` stay attached to `main.rs` as `#[cfg(test)] mod tests;` / `mod integration_tests;` and continue to `use super::*;` — preservation depends on the `#[cfg(test)] pub(crate) use crate::{...}` re-export block in `main.rs` enumerated in §6 of the plan.
  - Cargo.toml `[[bin]].path = "src/daemon/main.rs"` is frozen.
  - No new dependencies are introduced; the refactor is behaviour-preserving (no CLI/DBus/log-line changes).
