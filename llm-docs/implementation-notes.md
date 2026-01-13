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

## Testing

Not yet tested on actual environments. See LLM-TODO.md for testing checklist.
