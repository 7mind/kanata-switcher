# Implementation Notes

## Key Decisions

1. **Single Rust daemon for all environments** - Config logic shared, no duplication
2. **GNOME extension is minimal** - Only exposes DBus, daemon does the work
3. **KDE script injected at runtime** - No manual installation needed
4. **Auto-detect default layer** - On connect, Kanata sends initial `LayerChange`; daemon stores this as fallback when no rule matches
5. **GNOME extension auto-install by default** - Controlled by `--[no-]install-gnome-extension` flags

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
1. **Quick probe** - Try D-Bus call to extension. If responds → extension is active, skip all checks
2. **Fallback** - If not responding:
   - Check **installed** via `gnome-extensions info` (requires `XDG_DATA_DIRS`)
   - Check **enabled** via `gsettings get org.gnome.shell enabled-extensions` (works in systemd)

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

## Unfocus Handling

When all windows are closed (no window focused), the daemon switches to the default layer:

- **Wayland/COSMIC**: Protocol sets `active_window = None`, `get_active_window()` returns empty `WindowInfo`
- **GNOME**: Extension returns `{class: "", title: ""}` when `global.display.focus_window` is null
- **KDE**: KWin script calls with empty strings when `client` is null/undefined

`FocusHandler::handle()` detects empty class+title and returns `Some(default_layer)` to trigger the switch.

## X11 Backend

Uses x11rb with pure Rust connection (no libxcb dependency). Implementation in `run_x11()`:
1. Connect to X server via `x11rb::connect(None)` (reads $DISPLAY)
2. Get atoms for `_NET_ACTIVE_WINDOW`, `_NET_WM_NAME`, `UTF8_STRING`
3. Subscribe to `PropertyNotify` events on root window
4. Process initial focused window at startup
5. Event loop: wait for `PropertyNotify`, filter for `_NET_ACTIVE_WINDOW` changes

X11 is fallback - only used if GNOME/KDE/Wayland not detected.

## GNOME Extension (Push Model)

Extension subscribes to `global.display.connect('notify::focus-window')` and calls daemon's DBus `WindowFocus(class, title)` method on changes. Handles:
- Initial state: calls `_notifyFocus()` in `enable()`
- Unfocus: passes empty strings when `focus_window` is null

## Virtual Key Support

Two modes for virtual key actions:

1. **Simple mode (`virtual_key`)**: Auto-managed press/release
   - Pressed when window matches rule
   - Released when switching to different window
   - At most one VK active at a time
   - Tracked in `FocusHandler::current_virtual_key`

2. **Advanced mode (`raw_vk_action`)**: Fire-and-forget
   - Array of `[name, action]` pairs
   - Fired on focus only, no auto-release
   - Actions: `Press`, `Release`, `Tap`, `Toggle`

**Fallthrough**: Rules can set `fallthrough: true` to continue matching subsequent rules:
- First matching `layer` is used
- First matching `virtual_key` is used
- All matching `raw_vk_action` arrays are collected

**Execution order** (in `execute_focus_actions`):
1. Release previous `virtual_key` (if any)
2. Press new `virtual_key` (if any)
3. Fire all `raw_vk_action` pairs
4. Change layer (if specified)

## Testing

Tested on all supported environments:
- GNOME Shell (Wayland)
- KDE Plasma
- COSMIC
- Sway, Hyprland, Niri (wlr-foreign-toplevel-management protocol)
- X11 (various window managers)
