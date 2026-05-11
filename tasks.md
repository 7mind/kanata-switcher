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

- [ ] **PR-00** — Pre-flight: toolchain probe (`async fn` in trait + `wayland_scanner` macro path resolution).
- [ ] **PR-01** — Extract `constants.rs`, `errors.rs`, `env.rs`.
- [ ] **PR-02** — Extract `dbus_naming.rs` (+ `DbusSuffixError` into `errors.rs`).
- [ ] **PR-03** — Extract `config.rs` and `focus.rs`.
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
