# Implementation Notes

## Key Decisions

1. **Single Rust daemon for all environments** - Config logic shared, no duplication
2. **GNOME extension is minimal** - Only exposes DBus, daemon does the work
3. **KDE script injected at runtime** - No manual installation needed
4. **Auto-detect default layer** - On connect, daemon requests layer list; first layer (definition order) is used as fallback when no rule matches
5. **GNOME extension auto-install by default** - Controlled by `--[no-]install-gnome-extension` flags
6. **CLI control commands** - `--restart`, `--pause`, `--unpause` send DBus requests to an existing daemon and exit
7. **SNI indicator for non-GNOME** - StatusNotifier item with Pause/Restart and “Show app layer only” menu toggle (disable with `--no-indicator`)
8. **Autostart fallback** - `--install-autostart` writes a user autostart `.desktop` entry with the daemon args you passed (absolute Exec path); `--uninstall-autostart` removes it
9. **Runtime lifecycle supervision** - backend transitions are controlled in-daemon via lifecycle providers (logind continuous mode, startup-snapshot fallback mode)
10. **Supervisor wake-on-backend-finish** - active backend task completion is part of the supervisor wait set, preventing hangs when a backend exits without lifecycle signal changes
11. **SNI runtime-environment coherence** - local SNI pause/unpause uses runtime backend environment published by supervisor transitions, not fixed startup environment
12. **Wayland capability re-evaluation** - supervisor periodically re-resolves wayland backend flavor from last lifecycle snapshot to handle GNOME/KDE readiness races without waiting for new logind events
13. **Runtime-driven SNI lifecycle** - SNI control mode (local/dbus/off) is selected from current runtime environment transitions, not only startup environment detection
14. **Transient Wayland capability probe errors are non-fatal** - supervisor logs resolver failures, keeps the current backend alive, and retries on subsequent lifecycle/recheck ticks
15. **DE transitions rebuild SNI resources restart-style** - runtime SNI transition planning now restarts indicator resources on every environment change (even same control mode), making DE switches equivalent to restart semantics for indicator state/resources
16. **Logind monitor decode is process-fatal** - signal/decode failures in the detached logind listener now trigger explicit process termination (`exit(1)`), preventing silent degradation to stale lifecycle state
17. **Logind Type is required at provider startup** - initial logind session `Type` read now errors instead of defaulting to empty, and active-session empty `Type` is rejected to avoid silent `NoSession` idle startup
18. **Wayland capability polling is continuous-mode only** - periodic Wayland backend flavor rechecks now run only when lifecycle provider is continuous (login1); startup-snapshot mode remains strictly startup-only after its single snapshot
19. **Logind monitor stream health is fail-fast** - if the properties-changed stream terminates unexpectedly, the monitor now fails the process instead of silently degrading to stale lifecycle state
20. **Startup-snapshot resolver failures are mostly fatal** - in startup-only lifecycle mode, initial target resolution errors fail supervisor startup, except Wayland resolver/probe errors which now degrade to generic Wayland fallback for startup robustness
21. **Logind provider init is non-blocking and push-waited** - provider construction now returns immediately; when no display session exists yet, lifecycle monitor waits on login1 `User.Display` property changes (no retry polling), then starts session monitoring and emits initial snapshot
22. **Logind monitor reattaches after logout/login** - lifecycle monitor now watches `User.Display` while attached; when display session path changes, it rebinds to the new `Session` object and emits a fresh snapshot. This prevents stale idle state after GNOME logout/login cycles.
23. **Display-clear detaches stale session stream** - when `User.Display` becomes `/`, lifecycle emits `NoSession` (if needed) and drops the old session properties stream so normal session-object teardown cannot trigger fail-fast; monitoring resumes on next `User.Display` session path.
24. **DBus control service is lifecycle-independent and persistent** - DBus service registration moved out of per-backend tasks into a dedicated runtime manager task. It stays registered through idle/backend transitions, monitors `NameLost` push signals, and reconnects/re-registers after bus loss.
25. **KDE unpause query mode is runtime-probed** - DBus unpause no longer uses startup `KDE_SESSION_VERSION` to choose KDE5 vs KDE6 focus-query mode. It probes KWin script object-path layout (`/Scripting/ScriptN` vs `/N`) on the current runtime bus and selects query mode from that.
26. **Runtime SNI restarts do not leak watcher tasks** - indicator status/pause/menu watcher tasks are now owned by the active indicator runtime handle and aborted on indicator shutdown/drop, so repeated runtime Start/Restart transitions cannot accumulate orphaned watcher tasks.
27. **KDE backend startup query mode is runtime-probed** - `run_kde` now uses the same runtime KWin script-path probe as unpause, so persistent daemons transitioning into KDE sessions do not rely on stale startup `KDE_SESSION_VERSION`.
28. **GNOME extension setup follows runtime backend transitions** - GNOME extension setup moved to runtime transitions into GNOME backend, so persistent daemons that start outside GNOME still run setup when a GNOME session appears later.
29. **Runtime-managed SNI retries transient start failures** - when runtime SNI control construction fails (`build_sni_control_for_mode` returns `None`), the SNI manager no longer stalls until the next environment change; it retries on a fixed timer while still reacting to env-change signals.
30. **X11/Wayland backend startup refreshes display endpoints from logind** - before starting X11/Wayland backends, daemon resolves current display endpoint from active logind display session and passes explicit endpoints into backend connectors to avoid stale startup env dependence across logout/login transitions.
31. **KDE temp script paths are UUID-scoped** - KWin runtime/query/probe script filenames now include per-script UUIDs (in addition to existing query/probe counters where applicable), preventing cross-process `/tmp` collisions under parallel nextest runs and multi-instance execution.
32. **X11/Wayland unpause focus refresh reuses runtime display override resolver** - startup/unpause focus queries now use the same generalized logind display endpoint resolver path as backend startup, eliminating duplicate env-only connection logic and preventing stale `DISPLAY`/`WAYLAND_DISPLAY` failures after session transitions.
33. **Logind provider selection now gates on lifecycle-monitor prerequisites** - before selecting continuous logind mode, provider init now verifies login1 manager/user/session monitor prerequisites; failures in this phase fall back to startup-snapshot mode rather than returning a logind provider that later exits during detached monitor startup.
34. **Local SNI unpause is creation-context bound** - runtime-managed Local controls now store an explicit unpause context captured at control creation and do not read `runtime_environment.current()` at click time, preventing transition-race panics when environment flips to GNOME/KDE before control swap.
35. **Wayland GNOME flavor selection is owner-based** - runtime target resolution now selects GNOME backend whenever GNOME Shell owns its session bus name, without requiring extension focus-query readiness at selection time. This preserves startup-snapshot no-logind GNOME behavior; extension setup still runs on GNOME backend transition.
36. **Wayland override + startup probe fallback hardening** - login1 `Session.Display` overrides for Wayland are validated (rejecting X11-style values like `:0`), and startup-snapshot Wayland resolver/probe failures now fall back to generic Wayland target instead of terminating daemon startup.
37. **KDE runtime-mode probe waits for scripting interface readiness** - KDE startup/unpause runtime query-mode resolution now requires KWin ownership plus `/Scripting` introspection export of `org.kde.kwin.Scripting`, and retries probe attempts before failing. This prevents transient “KWin owned, scripting not ready yet” races from aborting runtime transition.
38. **Startup-snapshot preserves explicit GNOME/KDE identity** - no-logind startup snapshots now encode `session_type` hints (`gnome`/`kde`) and snapshot target resolution honors those hints directly. This removes one-shot owner-probe dependence for explicit GNOME/KDE startup environments and prevents permanent fallback to generic Wayland after transient early-session races.
39. **Continuous Wayland resolver errors are conditionally degraded** - in logind continuous mode, Wayland resolver failures now fallback to generic Wayland only when current target is non-Wayland-family (`idle`/`linux-console`/`x11`). If current target is already `gnome`/`kde`/`wayland`, supervisor keeps the active backend and retries on future rechecks, avoiding transient DBus/capability outages from tearing down working GNOME/KDE backends.
40. **Persistent DBus reconnect backoff now covers monitor-setup failures and is capped at 2s** - the persistent DBus manager applies the same bounded retry backoff path to session-bus connect, service registration, DBus proxy creation, and `NameLost` subscription setup failures. This removes rapid spin/log-flood loops when monitor setup fails while keeping reconnect latency responsive.
41. **Logind `User.Display` change decoding accepts structure encodings** - lifecycle monitor `User.Display` decode now accepts structure-wrapped object paths (including variant-wrapped structures), not only direct object-path/string values. This prevents false parse failures and fail-fast exits during real GNOME <-> Linux console display-session transitions.
42. **GNOME extension reconnects status after suspend/resume owner-loss races** - on daemon-owner loss, extension now keeps the last known layer/VK status (instead of resetting display state to empty) and starts a periodic owner probe timer. When owner becomes available again, it refreshes `GetStatus` and `GetPaused`, covering missed `notify::g-name-owner` recovery events after suspend.
43. **GNOME focus-only indicator falls back to last status when focus snapshot is empty** - `selectStatus()` now returns `lastStatus` if focus-only is enabled but focus layer is missing/invalid/blank, preventing lock/unlock and startup races from rendering `?` while daemon status is valid.

## Lifecycle Design Note

Problem observed in the field:
- If the daemon starts before any graphical login (common with lingering user services), startup can stall indefinitely while waiting for logind display-session readiness.
- Timeout-based startup failover avoids hanging but makes persistence depend on an external restart supervisor.

Target behavior (best design):
1. Lifecycle provider initialization must be non-blocking.
2. Supervisor should start immediately in Idle and remain process-persistent.
3. Logind integration should use push-based subscriptions (manager/user/session signal path), not startup polling loops.
4. Session monitoring should attach when display session appears and emit snapshots then.
5. Daemon should self-recover after arbitrarily long pre-login idle periods without requiring systemd restarts.

Status: implemented for login1-backed lifecycle via `User.Display` properties-changed wait path.

QA state: human testing status is tracked in `qa/`. Update those checklists after manual validation; they are part of the project state for LLM context.

## Rust Dependencies

Key crates:
- `zbus` - DBus for GNOME/KDE backends
- `wayland-client`, `wayland-protocols-wlr` - Wayland protocol handling
- `wayland-scanner` - generates COSMIC protocol bindings from XML
- `x11rb` - X11 protocol (pure Rust implementation)
- `tokio` - async runtime
- `clap` - CLI parsing
- `regex` - rule pattern matching
- `serde`, `serde_json` - config parsing

## Prior Art Referenced

Located in `./local/` (gitignored):
- `keymapper` - DBus push model, KWin script via kpackagetool
- `keyd` - FIFO push model, dynamic KWin injection via DBus
- `xremap` - DBus pull model with socket option
- `xremap-gnome` - GNOME extension exposing DBus
- `hyprkan` - Python daemon for Hyprland/Sway/Niri

keyd's approach for KDE was adopted: dynamic script injection via `org.kde.KWin.loadScript()`.

## GNOME Shell API

```javascript
// Get focused window
global.display.focus_window
window.get_wm_class()
window.get_title()

// DBus export
Gio.DBusExportedObject.wrapJSObject(xml, this)
this._dbus.export(Gio.DBus.session, '/path')
```

## GNOME Extension Detection

Detection flow (optimized for systemd services):
1. **Quick probe** - Call `org.gnome.Shell.Extensions.GetExtensionInfo` via D-Bus (native zbus). If extension state=1 (ENABLED) → active, skip all checks. This bypasses filesystem searches entirely.
2. **Startup retry** - If D-Bus returns state=6 (INITIALIZED), the extension is in the enabled list but GNOME Shell hasn't finished loading it yet. Retry every 50ms (up to 30s max) until state becomes ENABLED or changes.
3. **Fallback** - If D-Bus probe fails (GNOME Shell not running, no session bus):
   - Check **installed** via `gnome-extensions info` (requires `XDG_DATA_DIRS`)
   - Check **enabled** via `gsettings get org.gnome.shell enabled-extensions` (works in systemd)

Extension states: 1=ENABLED, 2=DISABLED, 3=ERROR, 4=OUT_OF_DATE, 5=DOWNLOADING, 6=INITIALIZED.

NixOS/Home Manager modules set `XDG_DATA_DIRS` environment for systemd services to ensure `gnome-extensions` can find Nix-installed extensions.

## KDE KWin API

```javascript
// KDE 6
workspace.windowActivated.connect(handler)
// KDE 5
workspace.clientActivated.connect(handler)

// Window properties
client.resourceClass  // window class
client.caption        // window title

// DBus call
callDBus(service, path, interface, method, ...args)
```

## Wayland Toplevel Protocols

The daemon uses standard Wayland protocols:

1. **wlr-foreign-toplevel-management** - works on wlroots compositors (Sway, Hyprland, Niri, etc.)
2. **cosmic-toplevel-info** - works on COSMIC (requires cosmic-workspace protocol as dependency)

Both protocols provide `title`, `app_id`, and `activated` state events. The daemon tries wlr first, falls back to cosmic.

## Kanata Reconnection

KanataClient handles disconnects automatically:
- Detects socket close/error events
- Exponential backoff: 1s → 2s → 5s (max)
- Queues pending layer change during disconnect, applies on reconnect
- Initial connection also retries with same backoff

## Shutdown

- Signal handler requests shutdown via a watch channel; backends exit cleanly on shutdown.
- Cleanup is handled in Drop guards (default layer reset, SNI shutdown, KWin script unload + temp file removal).

## Unfocus Handling

When all windows are closed (no window focused), the daemon switches to the default layer:

- **Wayland/COSMIC**: Protocol sets `active_window = None`, `get_active_window()` returns empty `WindowInfo`
- **GNOME**: Extension returns `{class: "", title: ""}` when `global.display.focus_window` is null
- **KDE**: KWin script calls with empty strings when `client` is null/undefined

`FocusHandler::handle()` detects empty class+title and returns `Some(default_layer)` to trigger the switch.

## Native Terminal Handling

When login1 is available, the daemon watches `org.freedesktop.login1.Session.Active` and `Type` on the system bus and transitions runtime targets internally:
- active `tty` -> Linux console backend
- active `wayland` -> GNOME/KDE/generic Wayland backend (owner-probed)
- active `x11` -> X11 backend
- inactive -> Idle (no focus backend running)

On Linux console activation it applies `on_native_terminal` focus actions; when returning to graphical sessions, backend startup performs initial focus sync.

Session resolution prefers `XDG_SESSION_ID`, otherwise `GetSessionByPID`. If the PID is not in a logind session (common for systemd user services with lingering), it falls back to the user’s `Display` session via `GetUserByPID` + `org.freedesktop.login1.User.Display`.
Logind replies are decoded by inspecting the reply signature (accepting `o`, `s`, `v`, or structures containing an object path) to tolerate object paths returned as a direct value, a structure (single- or multi-field), or a string.

If login1 is unavailable, the daemon falls back to startup-only provider mode: it picks one backend from startup env and does not continuously adapt to later lifecycle transitions.

## X11/Wayland Display Endpoint Handling

The X11/Wayland backends accept optional explicit display overrides at startup:
- X11: `x11rb::connect(display_override)`
- Wayland: explicit socket (`Connection::from_socket`) when override exists, otherwise `Connection::connect_to_env()`

### 1. logind-supported runtime (`LifecycleProvider::Logind`)

- On each runtime transition into X11/Wayland, supervisor resolves endpoint override from login1 before launching backend task.
- Resolution uses `resolve_logind_session_path` (`XDG_SESSION_ID` -> `GetSessionByPID` -> `User.Display`) and reads `Session.Type` + `Session.Display`.
- Override is used only when `Type` matches target backend (`x11`/`wayland`) and `Display` is non-empty.
- Wayland override values are validated before use; invalid values (for example, X11-style `:N`) are ignored and daemon falls back to env/default Wayland connection behavior.
- Wayland override handling:
  - absolute display string: treated as socket path directly
  - relative display string: resolved as `$XDG_RUNTIME_DIR/<display>`
- If resolution fails (login1 error, type mismatch, empty display), daemon logs and falls back to env-based connection for that backend start.

### 2. non-logind runtime (`LifecycleProvider::Startup`)

- Startup-only mode means no continuous login/session tracking.
- X11/Wayland startup attempts the same override resolver, but login1 is unavailable; fallback path is used:
  - X11: `x11rb::connect(None)` (from `DISPLAY`)
  - Wayland: `Connection::connect_to_env()` (from `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`)
- Startup-snapshot Wayland resolver/probe failures (for example, transient session-bus probe races) degrade to generic Wayland backend selection for that one-shot snapshot instead of failing process startup.
- After startup snapshot selection, no lifecycle-driven display refresh occurs in this mode.

## X11 Backend

Uses x11rb with pure Rust connection (no libxcb dependency). Implementation in `run_x11()`:
1. Connect to X server via `x11rb::connect(display_override)` (`Some(...)` from logind refresh when available, else `None` -> `$DISPLAY`)
2. Get atoms for `_NET_ACTIVE_WINDOW`, `_NET_WM_NAME`, `UTF8_STRING`
3. Subscribe to `PropertyNotify` events on root window
4. Process initial focused window at startup
5. Event loop: wait for `PropertyNotify`, filter for `_NET_ACTIVE_WINDOW` changes

X11 is fallback - only used if GNOME/KDE/Wayland not detected.

## GNOME Extension (Push Model + Pull API)

Extension subscribes to `global.display.connect('notify::focus-window')` and calls daemon's DBus `WindowFocus(class, title)` method on changes. Handles:
- Initial state: calls `_notifyFocus()` in `enable()`
- Unfocus: passes empty strings when `focus_window` is null

Top bar indicator:
- Optional panel indicator (settings key `show-top-bar-icon`) shows layer + virtual key status
- Extension listens for daemon `StatusChanged(layer, virtual_keys)` DBus signal and calls `GetStatus()` on startup
- Schemas must be compiled (`schemas/gschemas.compiled`) for `getSettings()` to work; build/install paths run `glib-compile-schemas`
- Preferences UI imports `ExtensionPreferences` from `resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js`
- Character formatting lives in `src/gnome-extension/format.js` with a GJS test in `tests/gnome-extension-format.js`
- DBus unpacking helper lives in `src/gnome-extension/dbus.js` with a GJS test in `tests/gnome-extension-dbus.js`
- Display format (GNOME + SNI):
  - Layer glyph: first letter of current layer (uppercase), `?` if empty/unknown.
  - Virtual keys glyph:
    - 0 VKs: no VK glyph shown.
    - 1 VK: show first letter of the VK name (uppercase).
    - 2–9 VKs: show the count.
    - >9 VKs: show `∞`.
- Status updates include a `source` field (`focus` or `external`); prefs default to showing focus-based layer only
- Focus updates force-broadcast via `StatusBroadcaster::update_focus_layer` so the indicator refreshes on focus events
- Indicator menu includes Pause, Settings, and Restart; Pause calls daemon DBus `Pause`/`Unpause`
- Pause handling releases managed virtual keys, switches to the default layer, disconnects from kanata, clears handler state, and ignores focus events for action execution
- The daemon proactively queries current focus on startup and unpause:
  - GNOME: extension exposes `GetFocus` over DBus (`com.github.kanata.Switcher.Gnome`).
  - KDE: daemon injects a one-shot KWin script that calls back over DBus with the current focus.
  - Wayland/X11: daemon queries the active window directly.
- GJS test also validates focus-only selection logic via `selectStatus()`

SNI indicator (non-GNOME):
- Optional StatusNotifier item for KDE/wlroots/COSMIC/X11; menu includes Pause/Restart and “Show app layer only”
- Uses the same layer + virtual key formatting as GNOME for counts 0–9; VK overflow renders as "9+" due to bitmap glyph limits
- Icon colors match GNOME: layer glyph white, VK glyph cyan
- Icon glyphs use Noto Sans Mono bitmap (size 32, basic Latin only); pause toggles through local handlers on non-DBus backends
- Tooltip: shows current layer; if any VKs are held, also lists the VK names (comma-separated)
- Persists "Show app layer only" via GSettings key `show-focus-layer-only` in schema `org.gnome.shell.extensions.kanata-switcher` when available; `--indicator-focus-only true|false` skips the GSettings read

## Virtual Key Support

Two modes for virtual key actions:

1. **Simple mode (`virtual_key`)**: Auto-managed press/release
   - All matching rules' VKs are pressed and held simultaneously
   - Released when focus changes and the VK is no longer matched
   - Tracked in `FocusHandler::current_virtual_keys` (Vec, preserves order)
   - VKs pressed in rule order (top-to-bottom), released in reverse order (bottom-to-top)

2. **Advanced mode (`raw_vk_action`)**: Fire-and-forget
   - Array of `[name, action]` pairs
   - Fired on focus only, no auto-release
   - Actions: `Press`, `Release`, `Tap`, `Toggle`

**Fallthrough**: Rules can set `fallthrough: true` to continue matching subsequent rules:
- ALL matching `layer`s execute in order, but **last wins** (kanata TCP `ChangeLayer` sets base layer, doesn't stack)
- ALL matching `virtual_key`s are pressed and held simultaneously (use with `layer-while-held` in kanata for stacking)
- All matching `raw_vk_action` arrays are collected

**FocusAction ADT**: Actions are represented as an algebraic data type:
- `ReleaseVk(name)` - Release a managed VK
- `ChangeLayer(layer)` - Switch to a layer
- `PressVk(name)` - Press and hold a managed VK
- `RawVkAction(name, action)` - Fire-and-forget VK action

**Execution order** (in `execute_focus_actions`):
1. Release VKs that are no longer matched (in reverse order of the old list)
2. For each matching rule in order:
   - Execute `layer` switch (if specified)
   - Execute `virtual_key` Press (if not already held)
   - Execute all `raw_vk_action` pairs

## DBus Backend (GNOME/KDE)

GNOME and KDE backends share a unified DBus service:
- `DbusWindowFocusService` struct with `window_focus(class, title)` method
- `register_dbus_service()` registers at `/com/github/kanata/Switcher`
- GNOME: register service, wait for extension to push events
- KDE: register service, inject KWin script, wait for script to push events

## Testing

**Manual testing** on all supported environments:
- GNOME Shell (Wayland)
- KDE Plasma
- COSMIC
- Sway, Hyprland, Niri (wlr-foreign-toplevel-management protocol)
- X11 (various window managers)

**Automated tests** in `src/daemon/tests.rs`:
- Flow tests: verify rule matching produces expected `FocusActions`
- Property tests (proptest): verify invariants like "release before press"
- Tests cover fallthrough, VK lifecycle, action ordering, edge cases
