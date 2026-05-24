# kanata-switcher Runtime Lifecycle Refactor Plan

## Document purpose

This document is a handover-ready implementation checklist for removing systemd restart glue and making `kanata-switcher` handle desktop/session lifecycle internally.

The target reader is a new agent that must implement this end-to-end with strong regression coverage.

## Problem breakdown

### Observed product issues

1. `try-restart kanata-switcher.service` is hardcoded and managed via a separate helper service.
2. Restart is only tied to graphical session start, not end.
3. Correctness currently depends on systemd wiring, because environment/backend detection is startup-only.

### Current code structure (as of this plan)

- Main startup orchestration: `src/daemon/main.rs` in `run_once()`.
- One-shot environment detection: `detect_environment()`.
- Runtime logind focus bridge: `start_logind_session_monitor()` + `apply_logind_focus()`.
- Backend entrypoints are long-running blocking loops:
  - `run_gnome`
  - `run_kde`
  - `run_wayland`
  - `run_x11`
  - `run_linux_console_with_logind`
- Module-level restart glue exists in `flake.nix`:
  - hardcoded command string for `systemctl --user try-restart ...`
  - helper unit `kanata-switcher-graphical-session-restart` in Home Manager module.
- NixOS integration in this repo also references helper service attributes in `modules/nixos/kanata-super-remap.nix`.

### Root cause

`kanata-switcher` currently selects one backend at startup and commits to it for the process lifetime. Session/desktop transitions are externalized to systemd restarts instead of handled as internal state transitions.

## Hard requirements and non-negotiable decisions

1. **Continuous lifecycle support only when login1 exists.**
   - `org.freedesktop.login1` (systemd-logind or elogind) is the only continuous lifecycle source.
2. **No periodic probing fallback for non-login1 environments.**
   - Without login1: startup-only detection remains, no continuous adaptation.
3. **Comprehensive tests are mandatory.**
   - Unit + integration + regression tests must cover transition logic and known failure modes.
4. **Fail fast.**
   - Invalid states or broken invariants should panic/exit, not silently degrade.
5. **Remove helper restart service logic after daemon-side lifecycle is complete.**

## Target architecture

## Lifecycle model

Introduce explicit runtime lifecycle domain model in `src/daemon/main.rs`:

- `SessionKind`:
  - `NoSession`
  - `GraphicalX11`
  - `GraphicalWayland`
  - `NativeTerminal`
- `DesktopFlavor`:
  - `Gnome`
  - `Kde`
  - `GenericWayland`
  - `X11`
  - `Unknown`
- `RuntimeTarget`:
  - `Backend(BackendKind)`
  - `Idle`
- `BackendKind`:
  - `Gnome`
  - `Kde`
  - `Wayland`
  - `X11`
  - `LinuxConsole`

## Lifecycle providers

Create a small provider abstraction:

- `LifecycleProvider` trait (or equivalent enum + functions) emitting lifecycle snapshots/events.
- Implementations:
  - `LogindLifecycleProvider`:
    - Uses `org.freedesktop.login1`.
    - Watches current session `Active` and `Type`.
    - Optional `Desktop` property usage where reliable.
  - `StartupSnapshotProvider`:
    - Emits one initial snapshot from current startup detection.
    - Emits nothing after startup.

Decision: provider selection is runtime capability-based.
- If system bus login1 connection and session resolution succeed: use `LogindLifecycleProvider`.
- Otherwise: use `StartupSnapshotProvider`.

## Backend supervisor

Add a supervisor loop that owns backend lifecycle:

- Maintains current `RuntimeTarget`.
- On new lifecycle event:
  - Compute next target deterministically.
  - If unchanged: no-op.
  - If changed: stop current backend, then start new backend, then perform initial focus sync.
- Stop/start operations must be explicit and awaited.

Required property:
- **At most one active focus backend at a time.**

## Backend adapters

Refactor current backend runners from terminal loops into controllable tasks:

- Extract each backend into:
  - `start_*_backend(...) -> BackendHandle`
  - `BackendHandle::stop() -> Result<()>`
- Backends must support external stop signal:
  - Existing `ShutdownHandle`/watch channel can be reused or wrapped per backend instance.

Keep current backend-specific logic intact where possible; only change lifecycle ownership.

## Target selection strategy

Given lifecycle state and available bus owners:

- `Type=tty` and active => `LinuxConsole`
- `Type=x11` and active => `X11`
- `Type=wayland` and active:
  - if GNOME owner present and extension interface ready => `Gnome`
  - else if KDE/KWin owner present => `Kde`
  - else => `Wayland`
- inactive => `Idle` (no focus backend running)

For startup-only provider:
- Compute one target and run it.
- No further transitions.

## Work checklist (concrete implementation)

## Phase 0: Baseline and safety

- [ ] Confirm existing tests are green (`nix develop -c cargo test`).
- [ ] Capture baseline logs for startup + logout + VT switch scenarios.
- [ ] Add temporary structured transition logging keys (single-line, parseable).

## Phase 1: Introduce lifecycle domain types

File: `src/daemon/main.rs`

- [ ] Add explicit lifecycle structs/enums.
- [ ] Add pure mapping functions:
  - `session_type_to_session_kind(...)`
  - `resolve_runtime_target(...)`
  - `target_requires_session_bus(...)`
- [ ] Keep these pure and testable.

## Phase 2: Extract provider layer

File: `src/daemon/main.rs`

- [ ] Implement `LogindLifecycleProvider` from existing logind monitor code.
- [ ] Implement `StartupSnapshotProvider` from existing startup detection path.
- [ ] Ensure `LogindLifecycleProvider` emits initial state before change stream loop.
- [ ] Ensure provider initialization fails fast and falls back only to startup-snapshot provider when login1 unavailable.
- [ ] Do not add any polling fallback.

## Phase 3: Refactor backend runners into handles

File: `src/daemon/main.rs`

- [ ] Wrap `run_gnome`, `run_kde`, `run_wayland`, `run_x11`, `run_linux_console_with_logind` in start/stop adapters.
- [ ] Add `BackendHandle` with owned join handle + stop signal.
- [ ] Make stop idempotent and awaited.
- [ ] Ensure teardown clears D-Bus registrations and avoids orphan tasks.

## Phase 4: Implement supervisor

File: `src/daemon/main.rs`

- [ ] Add supervisor state with current target and backend handle.
- [ ] Add transition function:
  - `transition(current, desired, context) -> new_state`.
- [ ] Integrate with provider stream.
- [ ] Remove one-shot `match env { ... run_* ... }` selection from `run_once`.
- [ ] Preserve control command, autostart install/uninstall, and signal handling semantics.

## Phase 5: Integrate existing focus/logind semantics

File: `src/daemon/main.rs`

- [ ] Reuse current `apply_logind_focus` semantics where valid.
- [ ] Ensure `session_type_indicates_native_terminal("tty")` stays authoritative.
- [ ] Keep no-login1 behavior startup-only and explicit in logs.

## Phase 6: Remove external restart glue

File: `worktrees/kanata-switcher/flake.nix`

- [ ] Remove hardcoded `graphicalSessionRestartCommand`.
- [ ] Remove `kanata-switcher-graphical-session-restart` HM unit.
- [ ] Keep only one primary `kanata-switcher` user service.

File: `modules/nixos/kanata-super-remap.nix` in parent repo

- [ ] Remove assumptions/references to helper unit (`kanata-switcher-graphical-session-restart`).
- [ ] Reassess activation restart workaround script:
  - Keep only if needed for deployment-time config churn.
  - Document why it remains if retained.

## Phase 7: Documentation updates

Files:
- `README.md` in `worktrees/kanata-switcher`
- module option descriptions in `flake.nix`

- [ ] Document login1-dependent continuous lifecycle.
- [ ] Document startup-only behavior without login1.
- [ ] Remove references to helper restart service.

## Comprehensive test plan (mandatory)

All tests below must be implemented unless an explicit blocker is documented.

## A. Pure unit tests (fast, deterministic)

File: `src/daemon/tests.rs`

- [ ] `session_type_to_session_kind` tests:
  - `tty`, `wayland`, `x11`, unknown strings.
- [ ] `resolve_runtime_target` matrix tests:
  - active/inactive × type × bus-owner combinations.
- [ ] Ensure no-env-special-casing regressions:
  - behavior must not depend on startup `Environment::Unknown` once lifecycle events are present.

## B. Supervisor transition tests

File: `src/daemon/tests.rs`

- [ ] No-op on same target.
- [ ] Stop old then start new ordering assertion.
- [ ] Transition `Idle -> Backend -> Idle`.
- [ ] Rapid sequence transitions without leaked handles.
- [ ] Fail-fast on backend start failure.

## C. Provider tests

File: `src/daemon/tests.rs` and/or `integration_tests.rs`

- [ ] `StartupSnapshotProvider` emits exactly one event.
- [ ] `LogindLifecycleProvider` decodes initial state correctly.
- [ ] `LogindLifecycleProvider` Active signal updates are propagated correctly.

## D. Integration tests with mock D-Bus services

File: `src/daemon/integration_tests.rs`

- [ ] GNOME owner appears/disappears => transitions between `Gnome` and fallback backend.
- [ ] KDE owner appears/disappears => transitions between `Kde` and fallback backend.
- [ ] `tty -> wayland -> tty` lifecycle switching.
- [ ] `wayland -> x11` switching.
- [ ] Ensure only one backend active by checking no duplicate event handling.

## E. Explicit regression tests for known issues

File: `src/daemon/tests.rs` and `src/daemon/integration_tests.rs`

- [ ] No false native-terminal activation on logout to greeter.
- [ ] No early GNOME shell misclassification when shell D-Bus appears before interface readiness.
- [ ] Unknown-startup + logind display session does not trigger terminal layer.
- [ ] Non-login1 path remains startup-only and does not attempt continuous transitions.

## F. Module/service tests (Nix-level)

Files:
- `worktrees/kanata-switcher/flake.nix` checks
- repo verification commands

- [ ] `nix flake check` (if applicable in worktree)
- [ ] `nix build /path/to/worktree#daemon --no-link`
- [ ] `./verify-configs --verbose <host>`
- [ ] If module edits touch shared paths, run wider `./verify-configs --verbose`.

## Acceptance criteria

Implementation is complete when all are true:

1. Single `kanata-switcher` user service provides correct lifecycle adaptation with login1.
2. No hardcoded `systemctl --user try-restart` helper logic remains in module outputs.
3. Session end/start transitions do not require external restart service.
4. Non-login1 mode is startup-only, explicitly documented, and tested.
5. Full test suite passes, including newly added lifecycle and regression tests.

## Risk register and mitigations

1. **Risk:** backend stop/start races leak resources.
   - **Mitigation:** explicit `BackendHandle` teardown contract + tests for no leaked tasks.
2. **Risk:** D-Bus owner checks are not sufficient readiness checks for GNOME.
   - **Mitigation:** keep existing probe-based readiness checks, not just name ownership.
3. **Risk:** complex transition storms during DM login/logout.
   - **Mitigation:** debounce/coalesce identical targets and enforce sequential transitions.
4. **Risk:** module consumers rely on helper service side effects.
   - **Mitigation:** remove helper service with companion Nix module updates and host verification.

## Handover execution order

Recommended order for a new agent:

1. Implement pure lifecycle mapping types/functions + tests.
2. Build supervisor skeleton with fake backend handles + transition tests.
3. Adapt real backends behind handles.
4. Integrate login1 provider.
5. Integrate startup-snapshot provider.
6. Remove helper restart glue from module outputs.
7. Add integration/regression tests.
8. Run full verification and prepare commits.

