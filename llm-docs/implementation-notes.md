# Implementation Notes

## Key Decisions

1. **Single Rust daemon for all environments** - Config logic shared, no duplication
2. **GNOME extension is minimal** - Only exposes DBus, daemon does the work
3. **KDE script injected at runtime** - No manual installation needed
4. **Auto-detect default layer** - On connect, Kanata sends initial `LayerChange`; daemon stores this as fallback when no rule matches
5. **GNOME extension auto-install by default** - Controlled by `--[no-]install-gnome-extension` flags
6. **CLI control commands** - `--restart`, `--pause`, `--unpause` send DBus requests to an existing daemon and exit
7. **SNI indicator for non-GNOME** - StatusNotifier item with Pause/Restart and “Show app layer only” menu toggle (disable with `--no-indicator`)
8. **Autostart fallback** - `--install-autostart` writes a user autostart `.desktop` entry with the daemon args you passed (absolute Exec path); `--uninstall-autostart` removes it

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

The daemon watches `org.freedesktop.login1.Session.Active` on the system bus. When the session becomes inactive (Ctrl+Alt+F*), it applies the `on_native_terminal` rule if present, otherwise it behaves like an unfocused state. When the session becomes active again, it refreshes focus by querying the backend (GNOME GetFocus DBus, KDE script callback, Wayland/X11 active-window query).

Session resolution prefers `XDG_SESSION_ID`, otherwise `GetSessionByPID`. If the PID is not in a logind session (common for systemd user services with lingering), it falls back to the user’s `Display` session via `GetUserByPID` + `org.freedesktop.login1.User.Display`.
Logind replies are decoded by inspecting the reply signature (accepting `o`, `s`, `v`, or structures containing an object path) to tolerate object paths returned as a direct value, a structure (single- or multi-field), or a string.

If logind monitoring fails to start (no system bus, permissions, etc.), the daemon logs the error and continues without native terminal switching.

## X11 Backend

Uses x11rb with pure Rust connection (no libxcb dependency). Implementation in `run_x11()`:
1. Connect to X server via `x11rb::connect(None)` (reads $DISPLAY)
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
- Uses the same layer + virtual key formatting as GNOME; pause toggles through local handlers on non-DBus backends
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
