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

All backends are event-driven (push model) - no polling.

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

Daemon captures first `LayerChange` as default layer (used when no rule matches).

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
- Disables auto-detection from Kanata
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
