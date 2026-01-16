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
    ┌──────────────┬──────────────┼──────────────┬──────────────┐
    ▼              ▼              ▼              ▼              ▼
┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐
│  GNOME  │   │   KDE   │   │ Wayland │   │   X11   │   │ Wayland │
│  DBus   │   │  KWin   │   │   wlr   │   │ x11rb   │   │ cosmic  │
└────┬────┘   └────┬────┘   └────┬────┘   └────┬────┘   └────┬────┘
     │             │             │             │             │
     ▼             ▼             ▼             ▼             ▼
┌─────────┐   ┌─────────┐   Sway,Hypr,   _NET_ACTIVE      COSMIC
│Extension│   │ Script  │   Niri,etc.    _WINDOW
│ (auto)  │   │ (auto)  │
└─────────┘   └─────────┘
```

## Backend Detection

| Environment | Detection | Method |
|-------------|-----------|--------|
| GNOME | `XDG_CURRENT_DESKTOP` contains "gnome" | DBus extension poll |
| KDE | `KDE_SESSION_VERSION` set | KWin script injection |
| Wayland | `WAYLAND_DISPLAY` set | Toplevel protocol (wlr or cosmic) |
| X11 | `DISPLAY` set | _NET_ACTIVE_WINDOW polling |

Detection order: GNOME → KDE → Wayland → X11 → Unknown

## Wayland Toplevel Protocol

The Wayland backend tries protocols in order:
1. `wlr-foreign-toplevel-management` - works on Sway, Hyprland, Niri, etc.
2. `cosmic-toplevel-info` - works on COSMIC

Both protocols provide `title`, `app_id`, and `activated` state events.

## X11 Backend

Uses x11rb (pure Rust X11 implementation, no libxcb dependency).

X11 atoms used:
- `_NET_ACTIVE_WINDOW` - get currently focused window
- `WM_CLASS` - get window class (returns `instance\0class\0`)
- `_NET_WM_NAME` - get window title (UTF-8, preferred)
- `WM_NAME` - get window title (fallback, Latin-1)

Polling-based like GNOME backend (100ms interval).

## Kanata Protocol

TCP JSON, newline-delimited. Default port 10000.

```
→ Server sends on connect:  {"LayerChange": {"new": "base"}}
← Client sends to switch:   {"ChangeLayer": {"new": "vim"}}
→ Server confirms:          {"LayerChange": {"new": "vim"}}
```

Daemon captures first `LayerChange` as default layer (used when no rule matches).

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
  {"class": "^firefox$", "layer": "browser"},
  {"class": "terminal", "title": "vim", "layer": "vim"}
]
```

**Rule entries:**
- `class`: regex against window class (optional)
- `title`: regex against window title (optional)
- `layer`: kanata layer name
- First match wins, default layer if no match

**Default entry (optional):**
- `{"default": "layer_name"}`: specifies explicit default layer
- Disables auto-detection from Kanata
- Can appear 0 or 1 times (multiple = error)
- Position in array doesn't matter

## GNOME Extension

Location: `src/gnome-extension/` (2 files: extension.js, metadata.json)

DBus service exposed by extension:
- Path: `/com/github/kanata/Switcher`
- Interface: `com.github.kanata.Switcher`
- Method: `GetFocusedWindow()` → `{"class": "...", "title": "..."}`

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
--install-gnome-extension    Auto-install GNOME extension (default)
--no-install-gnome-extension Skip auto-install
```

Systemd units use `--quiet` by default.

## Rust Dependencies

Key crates:
- `zbus` - DBus for GNOME/KDE backends
- `wayland-client`, `wayland-protocols-wlr` - Wayland protocol handling
- `wayland-scanner` - generates COSMIC protocol bindings from XML
- `x11rb` - X11 protocol (pure Rust, no libxcb dependency)
- `tokio` - async runtime
- `clap` - CLI parsing
