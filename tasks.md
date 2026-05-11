# kanata-switcher — Task Ledger

Authoritative ledger of planned and completed work for the `main.rs` refactor.
Project conventions: `CLAUDE.md` and `llm-docs/`.

Status: `[ ]` planned · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Milestones (high-level)

- [x] **M0** — Produce a refactor plan for `src/daemon/main.rs` into a trait-based multi-file architecture.
- [ ] **M1** — Execute PR-00..PR-15 from the plan, one PR at a time, behaviour-preserving. Not commissioned this session.

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
- [ ] **PR-04** — Extract `args.rs`, `autostart.rs`, `broadcasters.rs`, `kanata.rs` (splittable 4a/4b/4c).
- [ ] **PR-05** — Extract `control/{mod,client}.rs`.
- [ ] **PR-06** — Extract `pause.rs` and `focus_pipeline.rs`.
- [ ] **PR-07** — Extract `lifecycle/` (mod, startup, logind, snapshot helpers).
- [ ] **PR-08** — Extract `supervisor/` (mod, capabilities, display_override) — splittable 8a/8b.
- [ ] **PR-09** — Extract `backends/wayland/` including `wayland_scanner` protocol modules.
- [ ] **PR-10** — Extract `backends/x11.rs` and `backends/linux_console.rs`.
- [ ] **PR-11** — Extract `backends/gnome.rs` and `backends/kde/`.
- [ ] **PR-12** — Introduce `FocusBackend` trait; retire `run_*_backend_task` adapters.
- [ ] **PR-13** — Extract `control/server.rs` and `control/persistent.rs`.
- [ ] **PR-14** — Extract `sni/` (splittable 14a/14b/14c).
- [ ] **PR-15** — Extract `gnome_ext/` (with `embed.rs` relative-path bump).

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
