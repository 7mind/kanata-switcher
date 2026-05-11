# kanata-switcher — Defect Ledger

Audit trail of defects raised by adversarial review during the refactor.

Status: `[ ]` open · `[~]` under fix · `[x]` resolved

---

## PR-PLAN

### PR-PLAN-D01
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §7 risks item 5 (line 422); also §3 module tree and §5 PR-08/PR-09/PR-11.
- **Description**: The plan asserts the module graph is acyclic "by construction" with `supervisor` importing `backends::*` and `backends` importing nothing from `supervisor`. The actual code disagrees: `query_focus_for_env` (main.rs lines 2515–2544) calls `resolve_display_override_for_environment` at both lines 2530 and 2536. The plan moves `query_focus_for_env` to `backends/mod.rs` in PR-11, while `resolve_display_override_for_environment` is moved to `supervisor/display_override.rs` in PR-08. After PR-11, `backends/mod.rs` must `use crate::supervisor::display_override::resolve_display_override_for_environment;` — a backends → supervisor dependency the plan denies.
- **Root cause**: Inventory grouping was assigned by "feels like supervisor concern" without tracing call edges. `resolve_display_override_for_environment` is consumed from both supervisor (lifecycle path, lines 3692/3706) AND the focus-dispatch path (2530/2536). It is shared infrastructure, not supervisor-private.
- **Suggested fix**: In §3 and PR-08, move `resolve_display_override_for_environment`, `resolve_display_override_for_backend_kind`, `resolve_display_override_from_logind`, `display_override_expected_session_type`, `is_valid_wayland_display_override`, `normalize_display_override`, `display_override_backend_kind_for_environment`, and the test-only `*display_override*` slot/guard items to a shared location — either `env.rs` (extending it), a new top-level `display_override.rs`, or `backends/display_override.rs`. Keep `supervisor/` importing FROM that shared module, not exporting it. Update §7 risk-5 wording accordingly. Failing that, explicitly call out the backends→supervisor edge and explain how the cycle is avoided.

### PR-PLAN-D02
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-08 (line 266) and §6.10 (test plumbing).
- **Description**: The plan's PR-08 scope for `supervisor/display_override.rs` lists `TestFocusQueryDisplayOverrideGuard`, `display_override_test_slot`, the `set_test_focus_query_display_override` and `resolve_test_focus_query_display_override` variants, plus "test-only `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE`" — but main.rs lines 3559–3564 define **two** statics: `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE` (3560) AND `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` (3563). `display_override_test_slot` at line 3582 dereferences both. PR-10 (X11 backend) explicitly defers `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE` to display_override.rs but never mentions the Wayland sibling at all. PR-09 (wayland backend) similarly omits it. `integration_tests.rs` line 3668 calls `super::set_test_focus_query_display_override(...)` for Wayland — the missing static must travel with the function.
- **Root cause**: Plan author saw one static (`TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE`) and overlooked `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` two lines below.
- **Suggested fix**: Edit PR-08 scope (and §6.10 enumeration) to list `TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE` AND `TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE` (both at main.rs lines 3560 and 3563) as moving into `supervisor/display_override.rs`. Re-export both via the `#[cfg(test)] pub(crate) use ...` block (or include them under the `supervisor::*` glob in §6).

### PR-PLAN-D03
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-09 (lines 274–283); main.rs lines 65–79.
- **Description**: PR-09 moves `mod cosmic_workspace` and `mod cosmic_toplevel` from main.rs to `backends/wayland/protocols.rs`. Inside `mod cosmic_toplevel` at main.rs line 72 the code says `use crate::cosmic_workspace::__interfaces::*;` and at line 77 `use crate::cosmic_workspace::*;`. After the move, `crate::cosmic_workspace` no longer resolves — the modules are now `crate::backends::wayland::protocols::cosmic_workspace`. The plan calls out the `wayland_scanner` macro path concern but says nothing about updating these intra-module `crate::cosmic_workspace::*` paths.
- **Root cause**: Author probed macro path resolution but did not read the module body for absolute crate paths that break on relocation.
- **Suggested fix**: Add to PR-09 scope: "Update `use crate::cosmic_workspace::__interfaces::*;` (line 72) and `use crate::cosmic_workspace::*;` (line 77) inside `mod cosmic_toplevel` to use `super::cosmic_workspace::...` (relative within `protocols.rs`)." Also verify there is no other `crate::cosmic_*` or `crate::cosmic_toplevel::...` path anywhere in the moved bodies.

### PR-PLAN-D04
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §4.1 (lines 154–165); plan §1 "no new dependencies" (line 19).
- **Description**: The plan's `FocusBackend` example reads `#[async_trait] pub(crate) trait FocusBackend: Send + 'static { ... async fn run(...) -> Result<BackendExit, DynError>; }`. `#[async_trait]` is the `async-trait` crate's attribute macro — adding it requires a new dependency. The plan's §1 non-goals say "No new dependencies, no version bumps." The text then says "If the toolchain in use is pre-1.75 stabilisation, use `async-trait` — but that adds a dependency, which is banned." So the example contradicts the rule. The fallback "the trait method takes `-> impl Future<Output = ...> + Send` directly" is buried in a parenthetical. Separately, stable AFIT (Rust 1.75+) does **not** propagate `Send` from a `Send + 'static` supertrait to the returned future's auto-traits — so `async fn run(...) -> ...` in a trait does NOT yield a `Send` future by default. Spawning the backend with `tokio::spawn` requires a Send future. The plan glosses over this.
- **Root cause**: Author conflated AFIT stabilisation with Send-future ergonomics, and left a `#[async_trait]` annotation in the example that violates the no-new-deps rule.
- **Suggested fix**: Rewrite §4.1 to (a) remove the `#[async_trait]` annotation from the example, (b) commit to `fn run(self: Box<Self>, ctx: BackendRunContext) -> impl Future<Output = Result<BackendExit, DynError>> + Send;` (RPIT-in-traits, stable in Rust 1.75+ and confirmed available on the installed rustc 1.92.0), or alternatively `fn run(...) -> Pin<Box<dyn Future<Output = ...> + Send + '_>>;`. Drop the "If the toolchain is pre-1.75 use async-trait" branch — the project pins `rust-bin.stable.latest.default` in `flake.nix` line 32 and a `cargo --version` already shows 1.92.0, so the branch is dead. Add a verification step in PR-00: "compile a one-line trait with `fn run() -> impl Future<...> + Send` and confirm rustc accepts it."

### PR-PLAN-D05
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-09 (line 276); main.rs lines 2214 and 5364/5619.
- **Description**: PR-09 places `RawFdWatcher` (main.rs lines 2214–2228) inside `backends/wayland/mod.rs`. But `RawFdWatcher` is used by BOTH `run_wayland` (main.rs line 5364) AND `run_x11` (main.rs line 5619). After PR-10 moves X11 to `backends/x11.rs`, X11 would have to import `crate::backends::wayland::RawFdWatcher`, which is an x11→wayland coupling that has no semantic justification (the type is a generic FD wrapper).
- **Root cause**: Author placed `RawFdWatcher` with its first caller without checking the second caller in X11.
- **Suggested fix**: Move `RawFdWatcher` to `backends/mod.rs` (shared utility) — update PR-09 scope to remove it from `backends/wayland/mod.rs` and PR-10 scope to import from `crate::backends::RawFdWatcher`. Alternatively, place it in a new `src/daemon/fd.rs` if `backends/mod.rs` should remain trait-only.

### PR-PLAN-D06
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-08 (line 270); plan §3 (line 109 — "run_*_backend_task" sub-bullet); main.rs lines 3377–3458.
- **Description**: PR-08 says the five `run_*_backend_task` adapters in `supervisor/mod.rs` "leave referencing items in `main.rs` via `crate::run_gnome`, etc." But `run_gnome` (main.rs line 7225), `run_kde` (7445), `run_wayland` (5326), `run_x11` (5589) are all currently private (no `pub` modifier). The moment `run_gnome_backend_task` lives in `supervisor/mod.rs` and tries to call `crate::run_gnome(...)`, Rust will reject it with "function `run_gnome` is private". The plan does not flag the need to make these four free functions `pub(crate)` for the duration of PR-08 → PR-11, nor does it specify whether `query_gnome_focus`/`query_kde_focus`/`query_x11_active_window`/`query_wayland_active_window` (also called from the still-in-main `query_focus_for_env`) require the same temporary widening.
- **Root cause**: Plan reasoned about module placement but not about the visibility transitions required while items are temporarily split across the crate root and a sibling module.
- **Suggested fix**: Add to PR-08 a sub-step: "Before moving the supervisor body, change the four free functions `run_gnome`, `run_kde`, `run_wayland`, `run_x11` in main.rs from private to `pub(crate)` (so `crate::run_gnome` resolves from the new `supervisor/mod.rs`)." Same for any `query_*_focus`/`query_*_active_window` that remain in main.rs when `query_focus_for_env` (also still in main.rs) calls them after PR-09/PR-10. List the symbols explicitly in the PR's import-changes section.

### PR-PLAN-D07
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §3 module tree assignment of constants (lines 86, 91); plan §5 PR-01 (line 198); main.rs lines 6187–6192.
- **Description**: §3 says `constants.rs` holds "All shared string constants: DBUS_*, GNOME_FOCUS_*, KDE_*, LOGIND_*, KDE_KWIN_*, KDE_RUNTIME_QUERY_MODE_*." PR-01 lists only lines 91–125 plus GNOME_EXTENSION_SRC_PATH/SCHEMA. But main.rs lines 6187–6192 define `GNOME_SHELL_BUS_NAME`, `GNOME_SHELL_OBJECT_PATH`, `GNOME_SHELL_EXTENSIONS_INTERFACE`, `DBUS_ERROR_SERVICE_UNKNOWN`, `DBUS_ERROR_NAME_HAS_NO_OWNER`, `DBUS_ERROR_UNKNOWN_METHOD` — and `GNOME_SHELL_BUS_NAME` is referenced by `detect_desktop_capabilities` (line 3350), which PR-08 places in `supervisor/capabilities.rs`. After PR-15 moves the surrounding constants into `gnome_ext`, supervisor depends on a gnome_ext constant. The plan does not assign `GNOME_SHELL_BUS_NAME` to a destination, nor the three `DBUS_ERROR_*` constants. Likewise `BROADCAST_PER_CALL_TIMEOUT` (line 648), `NATIVE_TERMINAL_RULE_INDEX` (946), the eight `SNI_*` constants (1453–1464), `WAYLAND_CAPABILITY_RECHECK_INTERVAL` (3345), `AUTOSTART_DESKTOP_FILENAME` (346), `AUTOSTART_PASSTHROUGH_OPTIONS` (347), `AUTOSTART_ONESHOT_OPTIONS` (359), and `DBUS_RECONNECT_DELAYS_MS` (7563) are not in PR-01's listing.
- **Root cause**: Inventory captured constants only from the opening "Constants" cluster (91–125) and missed constants defined adjacent to their first consumer further down the file.
- **Suggested fix**: Update PR-01 (or a new PR sub-step) to enumerate every `const` in main.rs by line number and assign each to a module. Constants used cross-module → `constants.rs`. Constants used only by one cluster → that cluster's module (e.g. SNI_* → `sni/`, AUTOSTART_* → `autostart.rs`, BROADCAST_PER_CALL_TIMEOUT → `control/client.rs`, WAYLAND_CAPABILITY_RECHECK_INTERVAL → `supervisor/mod.rs`, DBUS_RECONNECT_DELAYS_MS → `control/persistent.rs`, GNOME_SHELL_* + DBUS_ERROR_* → either `constants.rs` or `gnome_ext/detection.rs` with re-export). Also map `GNOME_SHELL_BUS_NAME`'s consumer in `supervisor/capabilities.rs` to its destination explicitly.

### PR-PLAN-D08
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §2 inventory table (line 52); main.rs lines 4094–4145.
- **Description**: The inventory row "Lifecycle supervisor | 3203–4093" stops at line 4093, but `wait_for_wayland_capability_recheck` (line 4094), `wait_for_backend_completion_signal` (4114), and `poll_finished_backend_outcome` (4125) — all explicitly named in PR-08 scope — sit at lines 4094–4145. The inventory range under-counts the supervisor block by ~50 lines.
- **Root cause**: Range was clipped at a perceived section boundary.
- **Suggested fix**: Change the inventory row to "Lifecycle supervisor | 3203–4145" and re-validate the §2 table's LOC totals.

### PR-PLAN-D09
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §8 item 7 (line 442); main.rs line 1708.
- **Description**: Plan §8 admits "I didn't locate its exact definition line" for `SNI_RUNTIME_RETRY_INTERVAL`. The constant is at main.rs line 1708 (between `plan_sni_runtime_transition` at 1688 and `SniRuntimeWakeReason` at 1711). Plan §2 inventory does not list it either, although PR-14 does assign it to `sni/guard.rs`.
- **Root cause**: Stale TODO from drafting; never updated after a closer grep.
- **Suggested fix**: Replace §8 item 7 with a one-line confirmation: "`SNI_RUNTIME_RETRY_INTERVAL` is at main.rs line 1708, moves to `sni/guard.rs` in PR-14." Or delete §8 item 7 outright and add the constant to the §2 SNI inventory row.

### PR-PLAN-D10
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §5 PR-04 (line 230); main.rs spans 280–559, 1246–1464, 4246–4885.
- **Description**: PR-04 claims "LOC moved: ~960." Counting the four spans explicitly listed in PR-04: args+autostart 280–559 = 280 lines; broadcasters 1246–1464 = 219 lines; kanata-message-structs 4246–4317 = 72 lines; KanataClient 4319–4885 = 567 lines. Total ≈ 1138, **not** 960. PR-04 is ~18% over budget, not "borderline ~960". The plan's own success criterion "≤1200 LOC per source file" applies per destination, not per PR, but the PR-04 budget claim itself is misleading.
- **Root cause**: Estimate was rounded down without summing spans.
- **Suggested fix**: Update PR-04 to state "~1140 LOC moved" and reaffirm the §1 budget ("no source file >1200 LOC") applies to *each* destination file — `args.rs` ≈ 122 lines, `autostart.rs` ≈ 156 lines, `broadcasters.rs` ≈ 219 lines, `kanata.rs` ≈ 640 lines, all well under 1200. Then drop the "consider splitting" sentence or restate it as a reviewability concern, not a budget one.

### PR-PLAN-D11
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §5 PR-10 scope (line 287); main.rs lines 5452–5458.
- **Description**: PR-10 lists `X11State` + impl, `async fn run_x11`, `query_x11_active_window`, `run_x11_backend_task`, and the test static. It does NOT list the `x11rb::atom_manager!` macro invocation at lines 5452–5458 generating `X11Atoms`/`X11AtomsCookie`. This macro is X11-specific and used by `X11State` at line 5463 — must travel with the backend. An executor following the plan literally could leave it behind.
- **Root cause**: Inventory only captured `struct`/`enum`/`fn` items, not macro invocations.
- **Suggested fix**: Add to PR-10 scope: "Also move the `x11rb::atom_manager! { pub X11Atoms: X11AtomsCookie { ... } }` block at main.rs lines 5452–5458 into `backends/x11.rs` adjacent to `X11State`."

### PR-PLAN-D12
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §6 wayland_scanner paragraph (lines 392–393); plan §5 PR-09 (line 275); plan §7 risk 1 (line 414).
- **Description**: The plan asserts `wayland_scanner = 0.31.8`'s `generate_interfaces!`/`generate_client_code!` resolve XML paths "relative to `CARGO_MANIFEST_DIR` at expansion time", framing this as "almost certainly" the behaviour. The assertion is plausible but not actually verified against source — the plan says "PR-00 should include a one-line probe to confirm". This is the single highest-risk macro behaviour in the entire refactor, and the plan files it under §7 as a risk while still committing to PR-09 doing the move. If the macro turns out to be `Span::source_file()`-relative (some proc-macro crates pick this path on toolchain-dependent grounds), the fallback `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")` must be applied — but the plan does not describe how to revert PR-09 cleanly if the probe in PR-00 reveals incompatibility *after* PR-09 has already started.
- **Root cause**: Plan defers verification to PR-00 but writes PR-09 as if the verification succeeds, with no concrete decision tree for the failure branch.
- **Suggested fix**: Make PR-00 stronger — instead of an optional probe, mandate compiling a smoke test (`mod foo; ... wayland_scanner::generate_interfaces!("src/protocols/cosmic-workspace-unstable-v1.xml")` inside `foo` placed under `src/daemon/`) and only proceed to PR-09 once it compiles. If it fails, PR-09's scope must be amended *upfront* to use `concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocols/...")` for both macro calls. State this branch decision in PR-00, not as a §7 risk hedge.

### PR-PLAN-D13
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §5 PR-08 (lines 264–272) and §6 re-export block (lines 360–382).
- **Description**: The all-at-once `#[cfg(test)] pub(crate) use ...` re-export block in §6 includes `supervisor::*` but does not enumerate the test-only items inside `supervisor/display_override.rs`. Specifically `display_override_test_slot`, `set_test_focus_query_display_override`, `resolve_test_focus_query_display_override`, `TestFocusQueryDisplayOverrideGuard`, and the two test statics live inside a `pub(crate) mod display_override;` — they will be re-exported via `supervisor::*` only if `supervisor/mod.rs` itself has `pub(crate) use display_override::*;`. The plan does not state this is required, and integration_tests.rs at line 3668/4394 calls `super::set_test_focus_query_display_override` — if the re-export chain breaks, those tests fail to compile.
- **Root cause**: Plan documented only the outermost re-export and assumed inner-module glob re-exports happen by convention.
- **Suggested fix**: In §6 re-export block, change `supervisor::*` to `supervisor::{self, *, display_override::*, capabilities::*}` so each submodule's items reach the test scope. Or, add to PR-08 a requirement: `supervisor/mod.rs` must contain `pub(crate) use {display_override::*, capabilities::*};` so the outer `supervisor::*` glob captures the sub-module test-only items.

### PR-PLAN-D14
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §6 "wayland_scanner" (line 392) vs §1 non-goals "No converting … into a trait" — and the `mod cosmic_*` macro modules are wrapped in `#![allow(...)]` inner attributes (main.rs lines 52–54, 67–68).
- **Description**: Plan §7 risk 6 mentions `#![allow(...)]` inner attributes move with the module body. This is correct but understates the issue: the inner attributes also include `#![allow(missing_docs)]` and `#![allow(clippy::all)]` — moving these into `protocols.rs` means `clippy::all` is silenced ONLY for that file's contents at the module-body level, which is the intended scope. However, if `backends/wayland/protocols.rs` re-exports types via `pub use` or `pub mod`, lints fire at the use-site in `dispatch_*.rs`. The plan doesn't note that the cosmic protocol module re-exports (currently `use cosmic_toplevel::{...}; use cosmic_workspace::{...};` at main.rs lines 81–89) will need to be relocated to `dispatch_cosmic.rs` (per PR-09 line 282) and *those* import lines might fire clippy lints that were previously suppressed by the inner attribute on `mod cosmic_*`.
- **Root cause**: Plan handled the inner-attribute scoping correctly but did not trace the downstream import lines.
- **Suggested fix**: Add a note to PR-09: "After moving, run `cargo clippy --all-targets -- -D warnings` and audit any lint that fires inside `backends/wayland/dispatch_*.rs` on `use crate::backends::wayland::protocols::cosmic_*` lines. Suppress narrowly if needed; do NOT add `#![allow(clippy::all)]` to `dispatch_*.rs` — that hides real lints."

### PR-PLAN-D15
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §5 PR-12 (lines 306–312); plan §4.1 trait signature.
- **Description**: PR-12 says the trait method body is "the same code" as the existing `run_*_backend_task` adapters, so behaviour is identical. But the trait signature in §4.1 takes `self: Box<Self>` and `BackendRunContext` whereas the existing `async fn run_gnome(kanata, handler, status_broadcaster, restart_handle, pause_broadcaster, shutdown_handle)` takes six separate parameters and is called by `run_gnome_backend_task` which destructures a `BackendContext`. The trait conversion thus is NOT a pure relocation: it must (a) define `BackendRunContext` (a new struct), (b) construct one per backend at the supervisor call site, (c) field-by-field move each existing arg into the new struct, (d) update the `run_*` body if it referenced the args by name. The plan calls this "net ~0 LOC" — closer to net positive due to the struct definition and the per-backend `Box<Self>` boilerplate.
- **Root cause**: Plan underestimated the mechanical churn in converting positional args to a struct field bundle.
- **Suggested fix**: Restate PR-12 LOC as "+200 / -150 net +50" and add an explicit sub-step: "(1) define `pub(crate) struct BackendRunContext { ... }` in `backends/mod.rs` matching the existing `BackendContext` plus `ShutdownHandle`. (2) For each backend, add a small struct `XxxBackend` and an `impl FocusBackend for XxxBackend` whose `run` body unpacks `BackendRunContext` and delegates to the existing free `async fn run_xxx`. (3) In `supervisor::start_backend`, replace each `RuntimeTarget::Xxx => run_xxx_backend_task(...)` arm with `RuntimeTarget::Xxx => Box::new(XxxBackend::new(...)) as Box<dyn FocusBackend>`. (4) Delete `run_*_backend_task`."

### PR-PLAN-D16
- **Status**: `[x]` resolved
- **Severity**: nit
- **Location**: plan §5 PR-15 scope (line 333); main.rs lines 6037–6040.
- **Description**: PR-15 says "GNOME_EXTENSION_UUID const (currently at line 91 — already in `constants.rs` from PR-01; keep there or move; decision: leave in `constants.rs` since it's referenced from probe code that is now in `detection.rs`)". This is fine, but PR-01 §5 line 198 explicitly says "Moves to `constants.rs`: every `const` from lines 91–125 plus `GNOME_EXTENSION_SRC_PATH`, `GNOME_EXTENSION_SCHEMA_FILE`, `GNOME_EXTENSION_SCHEMA_COMPILED` from the GNOME ext block." `GNOME_EXTENSION_UUID` is at line 91 so it is covered by "lines 91–125". The PR-15 note is redundant but not wrong.
- **Root cause**: Minor doc redundancy.
- **Suggested fix**: Delete the parenthetical in PR-15 about GNOME_EXTENSION_UUID and replace with a single line: "GNOME_EXTENSION_UUID stays in `constants.rs` (already moved in PR-01)."

### PR-PLAN-D17
- **Status**: `[x]` resolved
- **Severity**: nit
- **Location**: plan §1 success criteria, line 11; plan §5 post-refactor sanity (line 343).
- **Description**: §1 sets `main.rs` final size as ≤ 350 LOC. Post-refactor sanity (line 346) says "`wc -l src/daemon/main.rs` reports < 400." Two different bars (350 vs 400).
- **Root cause**: Inconsistent budgets.
- **Suggested fix**: Pick one number (probably 400 for some headroom) and use it in both places.

### PR-PLAN-D18
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §3 module tree line 113 vs plan §5 PR-09 line 301.
- **Description**: The `§3` module tree describes `backends/mod.rs` as containing "`pub(crate) trait FocusBackend` (see §4). Also: `enum WaylandProtocol`, `RawFdWatcher`, `query_focus_for_env`, `apply_focus_for_env` (dispatch helpers shared by backends)." But PR-09's scope (line 301) places `WaylandProtocol` inside `backends/wayland/mod.rs`. The two entries directly contradict each other. `WaylandProtocol` is used only inside the wayland backend (main.rs lines 5321–5350) and naturally belongs in `backends/wayland/mod.rs`; the §3 entry is the wrong one.
- **Root cause**: Round-1 fix subagent updated PR-09 but failed to remove `WaylandProtocol` from the §3 line-113 listing for `backends/mod.rs`. Flagged in the subagent's parting note and still present.
- **Suggested fix**: In §3 line 113, delete `enum WaylandProtocol,` from the `backends/mod.rs` description. Keep `RawFdWatcher`, `query_focus_for_env`, `apply_focus_for_env` there. `WaylandProtocol` lives only in `backends/wayland/mod.rs` per PR-09.

### PR-PLAN-D19
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-06 (lines 268–275); main.rs lines 4184–4244 (`unpause_daemon` body) and 2546 (`apply_focus_for_env`).
- **Description**: PR-06 moves `pause_daemon` and `unpause_daemon` into `pause.rs`. `unpause_daemon` at main.rs line 4210 calls `apply_focus_for_env(...)`. PR-06's scope and "Import changes" do not mention widening `apply_focus_for_env` visibility, and `apply_focus_for_env` (line 2546) is currently a private free function in main.rs that does not move until PR-11 (when it goes to `backends/mod.rs`). For three PRs (PR-06 → PR-10) the new `pause.rs` would reference a private `crate::apply_focus_for_env`, producing "function `apply_focus_for_env` is private" at compile time. The same widening list in PR-08 (D06's resolution) names the four `run_*`/`query_*` functions but omits `apply_focus_for_env`, which has callers from BOTH `pause.rs` (after PR-06) and `supervisor::run_linux_console_backend_task` (after PR-08, main.rs line 3448).
- **Root cause**: The round-1 fix for D06 enumerated `run_*` and `query_*` functions but missed `apply_focus_for_env`, the highest-level focus-pipeline dispatcher that the supervisor's linux-console adapter and the pause module both depend on.
- **Suggested fix**: Add `apply_focus_for_env` (and, for safety, `query_focus_for_env`) to the visibility-widening sub-step. The widening must happen no later than PR-06 (the first PR that pulls a caller out of main.rs). Revert the widening in PR-11 when `apply_focus_for_env`/`query_focus_for_env` move to `backends/mod.rs`.

### PR-PLAN-D20
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §4.1 trait signature (line 158); plan §5 PR-12 (lines 332–345); plan §7 risk 5 (line 456).
- **Description**: The new `FocusBackend` trait in `backends/mod.rs` returns `Result<BackendExit, DynError>`. `BackendExit` is moved into `supervisor/mod.rs` in PR-08 (explicitly listed on line 290). Therefore `backends/mod.rs` must `use crate::supervisor::BackendExit;`, creating a backends → supervisor edge. §7 risk 5 still asserts "`supervisor` imports `backends::*` (to construct `FocusBackend` impls); `backends` import nothing from `supervisor`. This is acyclic by construction — keep it that way." That claim is false the moment PR-12 lands: each `XxxBackend::run` returns `BackendExit` and each backend file likewise imports it. Rust permits cyclic module-graph imports, but the plan's own invariant ("acyclic by construction") is violated. Worse, the per-backend `impl FocusBackend for XxxBackend` blocks each call `map_run_outcome_to_backend_exit` (per the §4.1 wording on line 166), which also lives in `supervisor/mod.rs` — a second supervisor symbol the backends consume.
- **Root cause**: `BackendExit` and `map_run_outcome_to_backend_exit` were classified as "supervisor" concerns while simultaneously being the trait's return type and the canonical conversion used by every trait implementor. The two roles are incompatible.
- **Suggested fix**: Move `BackendExit` (and the `RunOutcome` → `BackendExit` mapping helper, if it must travel with it) into a shared module — either `env.rs` (since `RunOutcome` already lives there) or a new `src/daemon/backend_exit.rs` sibling of `display_override.rs`. Update §3, PR-08, PR-12, and §7 risk 5 accordingly. Alternative: declare `BackendExit` in `backends/mod.rs` (the trait's natural home) and have `supervisor` import it from there — that flips the dependency direction but keeps the trait self-contained.

### PR-PLAN-D22
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §4.1 (line 164); plan §5 PR-12 step 2 (line 337).
- **Description**: §4.1 commits the trait declaration to `fn run(self: Box<Self>, ctx: BackendRunContext) -> impl Future<Output = Result<BackendExit, DynError>> + Send;` and explains why `+ Send` is mandatory. The plan does NOT spell out the impl-side signature. PR-12 step 2 (line 337) only says "add a small struct `XxxBackend` and `impl FocusBackend for XxxBackend` whose `run` body unpacks `BackendRunContext` and delegates to the existing free `async fn run_xxx`" — leaving the impl signature ambiguous. With RPIT-in-traits the impl must repeat the same `-> impl Future<...> + Send` form (or use `async fn run(...)` only if it auto-inherits Send from the captured futures, which is not guaranteed for futures that touch `Connection`/`KanataClient` clones). The PR-00 probe only validates the trait declaration shape, not an impl shape. An executor writing the impl as plain `async fn run(self: Box<Self>, ctx: BackendRunContext) -> Result<BackendExit, DynError> { ... }` may discover at PR-12 compile time that the impl's opaque return type is not `Send` and the `+ Send` trait bound is unsatisfied.
- **Root cause**: §4.1 documents the trait declaration but is silent on the impl signature contract.
- **Suggested fix**: Extend §4.1 (or PR-12 step 2) with the impl skeleton verbatim — e.g. `impl FocusBackend for GnomeBackend { fn run(self: Box<Self>, ctx: BackendRunContext) -> impl Future<Output = Result<BackendExit, DynError>> + Send { async move { /* unpack ctx and call run_gnome(...).await */ } } }`. Add a PR-00 sub-probe that compiles a one-line `impl` matching the trait to confirm `+ Send` propagation works on the installed rustc.

### PR-PLAN-D21
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §5 PR-06 (lines 268–275); main.rs lines 1613–1630, 6851/6873/6900.
- **Description**: `UnpauseContext` (main.rs line 1613) is moved to `pause.rs` in PR-06. It is also referenced by SNI code at line 1628 (the `SniLocalControl` field, which stays in main.rs until PR-14) and by the DBus server at lines 6851/6873/6900 (`resolve_runtime_unpause_context` and the `Unpause` method handler, which stay in main.rs until PR-13). PR-06's "Import changes" line only mentions importing `UnpauseContext` into `pause.rs` itself; it does not state that the SNI and DBus-server call sites still living in main.rs must switch to `use crate::pause::UnpauseContext;` in the same PR. Without that, `main.rs` retains five references to a now-moved type and the crate fails to compile.
- **Root cause**: Plan addressed the destination file's imports but not the source file's import updates for items that linger in main.rs after extraction.
- **Suggested fix**: Add to PR-06 import-changes: "main.rs gains `use crate::pause::UnpauseContext;` (or equivalent re-export through the `#[cfg(test)] pub(crate) use` block plus a non-test `pub(crate) use`) so existing SNI and DBus-server references at lines 1628/6851/6873/6900 continue to resolve until those clusters move in PR-14/PR-13." Apply the same pattern as a general rule for every PR that moves a cross-referenced type out before its consumers.

### PR-PLAN-D23
- **Status**: `[x]` resolved
- **Severity**: major
- **Location**: plan §5 PR-08 line 304 vs PR-12 step 0 line 350; main.rs lines 3245–3256 (`map_run_outcome_to_backend_exit` and `enum BackendExit`); supervisor cluster consumers at lines 3277, 3289, 3301, 3303, 3380, 3396, 3414, 3424, 3431, 3441, 3447, 3459, 3794, 3821, 4140–4141.
- **Description**: After the round-2 D20 fix, the plan contradicts itself about where `BackendExit` and `map_run_outcome_to_backend_exit` live between PR-08 and PR-12. PR-08 scope line 304 explicitly EXCLUDES them from the supervisor-move list ("everything else in 3203–4145 *except* … `BackendExit`/`map_run_outcome_to_backend_exit` (which belong in `backends/mod.rs` — see §4.1 and PR-12 below)"). But PR-08 does NOT create `backends/mod.rs` (PR-09 does), so the literal reading is: BackendExit stays in main.rs after PR-08. Yet PR-12 step 0 line 350 reads "Move `BackendExit` (main.rs lines 3203-area, currently in `supervisor/mod.rs` from PR-08) and `map_run_outcome_to_backend_exit` into `backends/mod.rs`." The "currently in supervisor/mod.rs from PR-08" parenthetical directly contradicts PR-08's exclusion clause. Concretely, after PR-08 lands as written, `supervisor/mod.rs` contains `BackendHandle` (line 3274, holds `JoinHandle<Result<BackendExit, DynError>>`), `take_join_result`/`stop` (lines 3289/3301, return `Result<BackendExit, DynError>`), the five `run_*_backend_task` adapters (lines 3380/3396/3414/3431/3447, return `Result<BackendExit, DynError>` and four of them call `map_run_outcome_to_backend_exit`), and `poll_finished_backend_outcome` (lines 4140–4141, matches on `BackendExit::Restart`/`Exit`). All of these reference a `BackendExit` that the plan leaves in main.rs as a private item. Compile result: "enum `BackendExit` is private" at every supervisor reference. PR-08's visibility-widening sub-step (line 305) lists `run_gnome`/`run_kde`/`run_wayland`/`run_x11` and the four `query_*` helpers but DOES NOT list `BackendExit` or `map_run_outcome_to_backend_exit`. The same gap blocks `run_linux_console_backend_task` (line 3447, moves to supervisor/) which calls `map_run_outcome_to_backend_exit` (line 3459).
- **Root cause**: D20 redirected `BackendExit` to `backends/mod.rs` to fix the acyclic-dependency invariant, but the plan never specified when `backends/mod.rs` first comes into existence relative to PR-08's supervisor extraction. PR-12 step 0's narration assumed BackendExit lives in `supervisor/mod.rs` after PR-08; PR-08 scope was edited to deny that. The two halves were not reconciled.
- **Suggested fix**: Pick one of three coherent options and apply it consistently across PR-08, PR-12 step 0, §3 module tree, and §7 risk 5:
  - **Option A (preferred)**: Have PR-08 also create an empty `backends/mod.rs` (just the `BackendExit` enum and `map_run_outcome_to_backend_exit` fn, no trait, no other items) so that PR-08's supervisor extraction can `use crate::backends::BackendExit;` from the outset. Add `mod backends;` to `main.rs` during PR-08. Update PR-12 step 0 to "add the `FocusBackend` trait and `BackendRunContext` to the already-existing `backends/mod.rs`" — drop the "move BackendExit" language.
  - **Option B**: Move `BackendExit` and `map_run_outcome_to_backend_exit` INTO `supervisor/mod.rs` during PR-08 (drop the exception clause from PR-08 scope line 304). Then PR-12 step 0 moves them out to `backends/mod.rs` as currently described. The transient backends→supervisor edge exists only between PR-08 and PR-12, with no backend module yet importing them.
  - **Option C**: Leave `BackendExit` and `map_run_outcome_to_backend_exit` in `main.rs` through PR-08 — but widen both to `pub(crate)` in PR-08's visibility-widening sub-step. Add their names to the list at line 305 ("`BackendExit`, `map_run_outcome_to_backend_exit`"). PR-12 step 0 then moves them from `main.rs` (not from supervisor) to `backends/mod.rs`. Revert the widening when PR-12 lands.
  Whichever option is chosen, update the PR-12 step 0 phrasing "currently in `supervisor/mod.rs` from PR-08" to match.

### PR-PLAN-D24
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: plan §6 re-export block (lines 408–431); plan §5 PR-11 (lines 336–344); plan §5 PR-09 (lines 312–324); main.rs tests usage of submodule items.
- **Description**: The `#[cfg(test)] pub(crate) use crate::{...}` re-export block enumerates `backends::{self, x11::*, gnome::*, kde::*, wayland::*, *}`. The `kde::*` and `wayland::*` globs reach only into `kde/mod.rs` and `wayland/mod.rs` directly — they do NOT transitively reach `kde/script.rs`, `kde/probe.rs`, `wayland/protocols.rs`, or `wayland/dispatch_*.rs`. Per PR-11, `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, and `build_kde_query_script` live in `backends/kde/script.rs`; `query_kde_focus` and `ensure_kde_scripting_ready` live in `backends/kde/probe.rs`. `tests.rs` lines 324/341/361/697 call `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, and `build_kde_focus_push_script` via `use super::*;` against `main.rs`. After PR-11, those symbols are not reachable through `kde::*` unless `kde/mod.rs` itself contains `pub(crate) use {script::*, probe::*};`. The plan does not state this requirement. Contrast with the sni entry on line 428 which explicitly lists every submodule with `::*` — the kde/wayland entries on line 413 are terser and incomplete. Same risk in `backends/wayland`: tests reference `wayland_query_count` (in `wayland/mod.rs`, OK), but any test that touches `protocols::cosmic_*` reaches deeper.
- **Root cause**: D13's round-1 fix enumerated supervisor submodules but did not apply the same enumeration to `backends/kde` and `backends/wayland` when those sub-trees were finalised in §3.
- **Suggested fix**: In PR-09, add to scope: "`backends/wayland/mod.rs` must contain `pub(crate) use {protocols::*, dispatch_common::*, dispatch_wlr::*, dispatch_cosmic::*};` (or, more narrowly, the specific items used by tests and downstream modules) so that the outer `backends::wayland::*` glob in the re-export block reaches submodule items." In PR-11, add the analogous: "`backends/kde/mod.rs` must contain `pub(crate) use {script::*, probe::*};` so that `kwin_query_script_path`, `kwin_query_probe_script_path`, `kwin_runtime_script_path`, `build_kde_query_script`, `query_kde_focus`, and `ensure_kde_scripting_ready` are reachable through `backends::kde::*` in `tests.rs`." Alternatively, expand the §6 re-export block to enumerate each submodule: `backends::{self, x11::*, gnome::*, kde::{self, script::*, probe::*, *}, wayland::{self, protocols::*, dispatch_common::*, dispatch_wlr::*, dispatch_cosmic::*, *}, *}`.

### PR-PLAN-D25
- **Status**: `[x]` resolved
- **Severity**: nit
- **Location**: plan §5 PR-08 line 306 ("LOC moved: ~850"); PR-12 line 356 ("+200 / -150 net +50").
- **Description**: After the D20 fix excluded `BackendExit` and `map_run_outcome_to_backend_exit` from PR-08's supervisor-move and shifted them to PR-12, the LOC accounting in both PRs drifted. The supervisor cluster (3203–4145) is ~942 LOC. PR-08 now moves ~929 LOC total (~746 to supervisor/, ~183 to display_override.rs), with ~13 LOC for `BackendExit`/`map_run_outcome_to_backend_exit` staying behind (until D23's resolution). The "~850" claim does not cleanly map to either the supervisor-only count (~746) or the combined count (~929). PR-12 line 356 says "+200 / -150 net +50", but PR-12 step 0 now also moves ~13 LOC of `BackendExit`/`map_run_outcome_to_backend_exit` into `backends/mod.rs`, nudging the net closer to +63.
- **Root cause**: LOC totals were not re-summed after the D20 reorganisation moved 13 LOC between PRs.
- **Suggested fix**: Update PR-08 LOC to "~930 (≈746 to `supervisor/`, ≈183 to `display_override.rs`)". Update PR-12 LOC to "+213 / -150 net +63 (including BackendExit and map_run_outcome_to_backend_exit moved out of main.rs/supervisor)". Minor — does not affect correctness, only the accuracy of the budget headers.

---

## PR-01

### PR-01-D01
- **Status**: `[x]` resolved
- **Severity**: minor
- **Location**: `src/daemon/environ.rs` (file name); `src/daemon/main.rs` lines 11 (`use std::env;`), 91-93 (`mod environ;`), 97 (`use environ::*;`), 101 (test re-export); plan §5 PR-01 line 230, §3 module tree.
- **Description**: The plan calls the new module `env.rs`. The executor named it `environ.rs` instead, to avoid colliding with `use std::env;` at main.rs line 11. The deviation is real: main.rs still calls `env::var(...)`/`env::var_os(...)` at lines 386, 392, 773, 2214, 2559 (5 call sites). Naming the new module `env` would shadow `std::env` and require either (a) removing `use std::env;` and rewriting each call site to `std::env::var(...)`, or (b) renaming the module on import (`use crate::env as environ;`), or (c) accepting the shadow and prefixing each `env::var` call with `std::`. The executor chose to rename the module instead. This works and tests pass, but the plan's prose still refers to `env.rs` / `crate::env::*` / `use crate::env::Environment;` in PR-01 §5, §3 module tree, and downstream PRs (PR-03 line 266, PR-04 line 277, PR-06 line 296, PR-07 line 306, PR-11 callouts).
- **Root cause**: The plan never probed the `std::env` shadowing case; PR-00 probe should have caught it. Executor made a defensible local fix without updating the plan.
- **Fix**: Accepted `environ` as the chosen name (lowest-friction option — keeps `use std::env;` intact, no per-call-site edits in main.rs). Updated plan to use `environ` consistently in §2 cross-cutting types row, §3 module tree (line 88), §3 main.rs `mod` declarations list (line 145), §5 PR-01 (lines 230, 245, 248), and the downstream PR sections that previously referenced `crate::env::*` (PR-03 line 266, PR-04 line 277, PR-06 line 296, PR-07 line 306). No code changes.
