# Architecture

## Overview

Single Rust daemon (`src/daemon/`) handles all desktop environments. Auto-detects environment via env vars.

```
                    ┌─────────────────────────────┐
                    │       Daemon (Rust)         │
                    │  - Config loading           │
                    │  - Rule matching            │
                    │  - Kanata TCP client        │
                    └─────────────┬───────────────┘
                                  │
    ┌─────────────────────┬───────┼───────┬──────────────┐
    ▼                     ▼       ▼       ▼              ▼
┌─────────────────┐   ┌───────┐ ┌───────┐ ┌───────┐  ┌───────┐
│  DBus Backend   │   │Wayland│ │Wayland│ │  X11  │  │       │
│  (GNOME + KDE)  │   │  wlr  │ │cosmic │ │ x11rb │  │       │
└────────┬────────┘   └───┬───┘ └───┬───┘ └───┬───┘  │       │
         │                │         │         │      │       │
    ┌────┴────┐           ▼         ▼         ▼      │       │
    ▼         ▼       Sway,etc.  COSMIC   _NET_ACTIVE│       │
┌───────┐ ┌───────┐                       _WINDOW    │       │
│ GNOME │ │  KDE  │                                  │       │
│  Ext  │ │ KWin  │                                  │       │
│(auto) │ │Script │                                  │       │
└───────┘ └───────┘                                  └───────┘
```

## Backend Detection

| Environment | Detection | Method |
|-------------|-----------|--------|
| GNOME | `XDG_CURRENT_DESKTOP` contains "gnome" | Shared DBus backend, extension pushes |
| KDE | `KDE_SESSION_VERSION` set | Shared DBus backend, KWin script pushes |
| Wayland | `WAYLAND_DISPLAY` set | Toplevel protocol events (wlr or cosmic) |
| X11 | `DISPLAY` set | PropertyNotify events on _NET_ACTIVE_WINDOW |

Detection order: GNOME → KDE → Wayland → X11 → Unknown

Startup env detection is now a fallback path. Runtime backend ownership is supervised by a lifecycle controller:
- Provider `logind` (continuous): when `org.freedesktop.login1` is available, session `Active`/`Type` events drive backend transitions (`tty`/`wayland`/`x11` + idle).
- Logind provider selection validates lifecycle-monitor prerequisites (manager/user/session monitor setup) before commit; if those checks fail, daemon does not enter continuous mode.
- In pre-login startup (no display session yet), logind lifecycle monitor waits on login1 `User.Display` property changes and attaches when the session appears; daemon startup remains non-blocking.
- During runtime, logind lifecycle monitor also tracks `User.Display` path changes and reattaches to the new display session object after logout/login cycles.
- When `User.Display` clears to `/`, session monitoring is detached until a non-empty display session path appears again.
- Provider `startup-snapshot` (single event): when login1 is unavailable or logind lifecycle-monitor prerequisites fail, the daemon runs startup-only backend selection.
- In startup-snapshot mode, explicit startup env detection for GNOME/KDE is preserved as snapshot intent (`session_type` hint), so startup target resolution does not rely solely on a one-shot session-bus owner probe.

No polling fallback is used when login1 is unavailable.

Backends are event-driven but the daemon performs one-shot focus queries on startup and unpause:
- GNOME: extension provides GetFocus over DBus
- KDE: daemon injects a one-shot KWin script and receives a DBus callback; both backend startup and unpause runtime mode selection probe KWin script object path layout at runtime (`/Scripting/ScriptN` for KDE6, `/N` for KDE5) instead of relying on startup env vars. Runtime mode probing now waits for KWin `/Scripting` export of `org.kde.kwin.Scripting` and retries, so transient startup races do not abort KDE backend startup.
- Wayland/X11: daemon queries the active window directly; startup/unpause focus queries now resolve display endpoint override via the same runtime logind refresh path used by backend startup, so they do not depend on stale startup `WAYLAND_DISPLAY`/`DISPLAY` after session endpoint changes
- Wayland flavor resolution picks GNOME when GNOME Shell owns its session bus name (owner-based selection), so startup-snapshot mode does not depend on extension focus-query readiness to enter GNOME backend
- GNOME extension setup (`setup_gnome_extension`) is executed from runtime transitions into the GNOME backend, so persistent daemons that start pre-login and later enter GNOME still install/enable/check the extension at the correct time.
- On X11/Wayland backend starts, daemon refreshes display endpoints from the current logind display session and connects with explicit endpoints (instead of depending solely on stale startup `DISPLAY`/`WAYLAND_DISPLAY`).

### X11/Wayland Display Endpoint Resolution

#### 1. logind-supported runtime (`LifecycleProvider::Logind`)

- Lifecycle transitions (`Idle <-> x11/wayland`) are continuous and login1-driven.
- Every runtime transition that starts X11/Wayland (`start_backend`) re-resolves endpoint data from login1 before connecting.
- Resolution path:
  - resolve active display session path (`resolve_logind_session_path`, with `XDG_SESSION_ID`/PID/User.Display fallback logic),
  - read `Session.Type` and `Session.Display` from login1,
  - accept override only when `Type` matches backend target (`x11` for X11, `wayland` for Wayland) and `Display` is non-empty.
- Wayland override values are validated before use; invalid values (for example `:0`) are ignored so backend start/focus query falls back to normal env/default Wayland connection.
- Backend connection behavior when override is accepted (no process env mutation; endpoint is passed explicitly to connector):
  - X11: `x11rb::connect(Some(display))`
  - Wayland: connect via `UnixStream` to explicit socket and build `wayland_client::Connection::from_socket`
    - absolute `Display` path is used as-is,
    - relative `Display` name resolves to `$XDG_RUNTIME_DIR/<Display>`.
- If logind refresh cannot provide a usable endpoint (query error, mismatch, empty display), daemon logs and falls back to standard env-based connector behavior for that start.

#### 2. non-logind runtime (`LifecycleProvider::Startup`)

- login1 is unavailable, so daemon runs startup-snapshot mode only (single backend selection from startup conditions).
- X11/Wayland startup still attempts the same override resolution call, but login1 access fails in this mode and the daemon falls back to env-based connectors:
  - X11: `x11rb::connect(None)` (uses `DISPLAY`)
  - Wayland: `Connection::connect_to_env()` (uses `WAYLAND_DISPLAY` + `XDG_RUNTIME_DIR`)
- If initial Wayland desktop-capability resolver/probe fails in startup-snapshot mode (for example, transient session-bus startup race), daemon falls back to generic Wayland target for that startup snapshot instead of exiting.
- Because provider is startup-only, daemon does not continuously re-resolve display/session state after startup in this mode.

Runtime-managed SNI indicator restarts own their watcher tasks (status/pause/menu) via an indicator handle wrapper; when the indicator is stopped or replaced, those tasks are aborted with the old handle to avoid task leaks across runtime transitions. If control construction fails transiently (for example, session bus race), runtime-managed SNI now retries with a timer and still wakes immediately on environment changes.
For Local SNI controls, unpause uses a context captured at control creation (env + focus-query context), not `runtime_environment.current()` at click time, to avoid transition races where stale Local controls observe GNOME/KDE without a matching session connection.

DBus control API (`com.github.kanata.Switcher`) is managed by a dedicated persistent task (not backend-owned):
- remains registered while session bus is available, including lifecycle `Idle`
- receives `NameLost` push signals and re-registers on bus/name loss
- retries session-bus connect/register with bounded backoff when bus is unavailable

## Wayland Toplevel Protocol

The Wayland backend tries protocols in order:
1. `wlr-foreign-toplevel-management` - works on Sway, Hyprland, Niri, etc.
2. `cosmic-toplevel-info` - works on COSMIC

Both protocols provide `title`, `app_id`, and `activated` state events.

## X11 Backend

Uses x11rb (pure Rust X11 implementation, no libxcb dependency).

Event-driven via PropertyNotify on root window:
1. Subscribe to `PROPERTY_CHANGE` events on root
2. Filter for `_NET_ACTIVE_WINDOW` atom changes
3. Process initial state on startup

X11 atoms used:
- `_NET_ACTIVE_WINDOW` - get currently focused window
- `WM_CLASS` - get window class (returns `instance\0class\0`)
- `_NET_WM_NAME` - get window title (UTF-8, preferred)
- `WM_NAME` - get window title (fallback, Latin-1)

## Kanata Protocol

TCP JSON, newline-delimited. Default port 10000.

```
→ Server sends on connect:  {"LayerChange": {"new": "base"}}
← Client sends to switch:   {"ChangeLayer": {"new": "vim"}}
→ Server confirms:          {"LayerChange": {"new": "vim"}}
← Client sends VK action:   {"ActOnFakeKey": {"name": "vk_browser", "action": "Press"}}
```

Daemon auto-detects default layer from first entry in kanata's layer list (definition order).

VK actions: `Press`, `Release`, `Tap`, `Toggle`.

### Reconnection

KanataClient handles disconnects automatically:
- Detects socket `close`/`error` events
- Exponential backoff: 1s → 2s → 5s (max)
- Queues pending layer change during disconnect, applies on reconnect
- Initial connection also retries with same backoff

### Shutdown

Daemon switches to default layer on exit (any cause):
- Signal handlers catch SIGTERM, SIGINT, SIGHUP
- `ShutdownGuard` (Drop impl) handles panics and normal exits
- Uses existing connection only, no reconnection attempt during shutdown
- Skips if not connected or default layer unknown

## Config Format

`~/.config/kanata/kanata-switcher.json`:
```json
[
  {"default": "default"},
  {"class": "^firefox$", "layer": "browser", "virtual_key": "vk_browser"},
  {"class": "terminal", "title": "vim", "layer": "vim"}
]
```

**Rule entries:**
- `class`: regex against window class (optional)
- `title`: regex against window title (optional)
- `on_native_terminal`: layer to switch to when active session is a native terminal (optional)
- `layer`: kanata layer name (optional)
- `virtual_key`: auto-managed VK - press on focus, release on unfocus (optional)
- `raw_vk_action`: array of `[name, action]` pairs, fire-and-forget on focus (optional)
- `fallthrough`: continue matching subsequent rules (default false)
- A matching rule with `fallthrough: false` stops evaluation; `fallthrough: true` continues
- Non-matching rules are skipped regardless of their fallthrough setting
- All matching rules' actions execute in order (layers, VKs, raw actions)
- Intermediate `virtual_key`s are tapped, final is held
- Default layer used if no match

**Default entry (optional):**
- `{"default": "layer_name"}`: specifies explicit default layer
- When absent, auto-detected from first layer in kanata's layer list (definition order)
- Can appear 0 or 1 times (multiple = error)
- Position in array doesn't matter

**Native terminal rule (optional):**
- `{"on_native_terminal": "layer_name"}`: applies when session switches to a native terminal (Ctrl+Alt+F*)
- Can appear 0 or 1 times (multiple = error)
- Must not include `class`, `title`, or `layer`
- Can include `virtual_key` and `raw_vk_action`

**Virtual key modes:**
- Simple (`virtual_key`): at most one VK active, auto-released on unfocus/switch
- Advanced (`raw_vk_action`): multiple actions, fire-and-forget
- Both can be used in same rule

## GNOME Extension

Location: `src/gnome-extension/` (`extension.js`, `prefs.js`, `metadata.json`, `schemas/`)

Behavior:
- Pushes focus changes to daemon DBus `WindowFocus(class, title)`
- Listens for daemon `StatusChanged(layer, virtual_keys, source)` signals
- Calls daemon `GetStatus()` on startup to populate the top bar indicator
- GSettings key `show-top-bar-icon` (schema `org.gnome.shell.extensions.kanata-switcher`) toggles the indicator
- GSettings key `show-focus-layer-only` controls whether external kanata layer changes are ignored
- Panel menu includes Pause, Settings, and Restart (Pause calls daemon DBus `Pause`/`Unpause`)

### Extension Loading

The daemon loads extension files from (in order):
1. **Filesystem**: `<exe-dir>/gnome/` (populated by build.rs or Nix)
2. **Embedded**: Compiled into binary via `include_str!` (if `embed-gnome-extension` feature enabled)

Cargo feature `embed-gnome-extension` (default: enabled):
- Enables fallback to embedded extension when filesystem copy not found
- Disabled in Nix builds (extension bundled alongside binary)

### Auto-install

Uses `gnome-extensions` CLI:
1. `gnome-extensions pack` → temp zip
2. `gnome-extensions install --force`
3. `gnome-extensions enable`
4. User must restart GNOME Shell

Controlled by `--[no-]install-gnome-extension` flag.

## KDE KWin Script

Generated at runtime, not a separate file. Injected via DBus:

```javascript
function notifyFocus(client) {
  if (!client) return;
  callDBus("com.github.kanata.Switcher", "/com/github/kanata/Switcher",
           "com.github.kanata.Switcher", "WindowFocus",
           client.resourceClass, client.caption);
}
workspace.windowActivated.connect(notifyFocus);  // KDE 6
notifyFocus(workspace.activeWindow);             // process current window at startup
```

KDE 5 uses `clientActivated`/`activeClient` instead of `windowActivated`/`activeWindow`.

Daemon exports DBus listener, KWin script pushes focus changes to it.

## Nix Flake

Packages:
- `daemon` - Rust daemon built with crane, auto-install enabled
- `gnome-extension` - stdenv derivation for Nix-managed install

NixOS module (`nixosModules.default`) - system-wide install with user service:
```nix
services.kanata-switcher = {
  enable = true;
  kanataPort = 10000;
  kanataHost = "127.0.0.1";
  configFile = null;  # defaults to ~/.config/kanata/kanata-switcher.json
  gnomeExtension.enable = false;  # installs + enables via dconf for all users
  gnomeExtension.autoInstall = false;
};
```

Creates `systemd.user.services.kanata-switcher` (starts for all users on graphical login).

Home Manager module options:
```nix
services.kanata-switcher = {
  enable = true;
  kanataPort = 10000;
  kanataHost = "127.0.0.1";
  configFile = null;  # defaults to ~/.config/kanata/kanata-switcher.json
  gnomeExtension.enable = false;     # Nix-managed extension
  gnomeExtension.autoInstall = false; # Runtime auto-install
};
```

HM module adds `--no-install-gnome-extension` by default. Use either:
- `gnomeExtension.enable = true` for Nix-managed (recommended)
- `gnomeExtension.autoInstall = true` for mutable runtime install

The old user helper service `kanata-switcher-graphical-session-restart` was removed; lifecycle transitions are handled in-daemon.

## CLI Options

```
-p, --port PORT              Kanata TCP port (default: 10000)
-H, --host HOST              Kanata host (default: 127.0.0.1)
-c, --config PATH            Config file path
-q, --quiet                  Suppress focus/layer-switch messages
--quiet-focus                Suppress focus messages only
--install-gnome-extension    Auto-install GNOME extension (default)
--no-install-gnome-extension Skip auto-install
```

Systemd units use `--quiet-focus` by default.

Nix module option `services.kanata-switcher.logging` controls the systemd unit logging flag:
- `quiet` -> `--quiet`
- `quiet-focus` -> `--quiet-focus` (default)
- `none` -> no quiet flags

## Testing

Test files:
- `src/daemon/tests.rs` - Unit tests for FocusHandler (rule matching, VK lifecycle)
- `src/daemon/integration_tests.rs` - Integration tests for DE backends

Integration tests:
- **DBus tests**: Test GNOME/KDE backend with mock Kanata TCP server
- **Wayland tests**: Mock compositor implementing wlr-foreign-toplevel-management
- **X11 tests**: Xvfb-based tests for PropertyNotify and window property reading

Running tests:
```bash
cargo test                   # All tests - requires Xvfb and dbus-daemon
xvfb-run cargo test          # With X11 display (if not in devShell)
nix run .#test               # Recommended: always runs tests via nextest
```

**How it works**: `nix run .#test` executes tests using cargo-nextest. The test archive is compiled once (cached via `cargo nextest archive`), but execution happens fresh every run. `nix flake check` reuses the same nextest archive.

**X11 test parallelism**: Each X11 test uses a unique hardcoded Xvfb display number (:100, :101, :102) to allow parallel execution with nextest (which spawns separate processes per test). See `XvfbGuard::start()` in `integration_tests.rs`.

Tests requiring external dependencies (Xvfb, dbus-daemon) fail with helpful error messages when unavailable.

## Rust Dependencies

Key crates:
- `zbus` - DBus for GNOME/KDE backends
- `wayland-client`, `wayland-protocols-wlr` - Wayland protocol handling
- `wayland-scanner` - generates COSMIC protocol bindings from XML
- `x11rb` - X11 protocol (pure Rust, no libxcb dependency)
- `tokio` - async runtime
- `clap` - CLI parsing

Dev dependencies:
- `proptest` - Property-based testing for FocusHandler
- `wayland-server` - Mock compositor for Wayland tests
