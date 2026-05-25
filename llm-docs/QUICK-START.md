# Quick Start for LLM Successors

**IMPORTANT**: Do NOT add line numbers to this documentation. They are unimportant details that easily go out of sync with the actual code. Use function/struct names and `grep` to locate code.

## Project Summary

Kanata layer switcher daemon - switches keyboard layers based on focused window. Single Rust daemon supports Linux (GNOME, KDE, Wayland, X11), macOS (NSWorkspace), and Windows (WinEvent hook).

## Key Files

- `src/daemon/main.rs` - Rust daemon (all backends)
- `src/daemon/backends/macos.rs` - macOS FocusBackend (NSWorkspace notifications)
- `src/daemon/backends/windows.rs` - Windows FocusBackend (WinEvent foreground hook)
- `src/daemon/platform/linux.rs` - Linux platform entry (supervisor, logind, SNI, DBus)
- `src/daemon/platform/macos.rs` - macOS platform entry (signal + MacOsBackend)
- `src/daemon/platform/windows.rs` - Windows platform entry (signal + WindowsBackend)
- `src/daemon/platform/mod.rs` - Platform dispatch
- `src/gnome-extension/` - GNOME Shell extension (bundled, auto-installed)
- `src/protocols/` - Wayland protocol XMLs (cosmic-toplevel-info, cosmic-workspace)
- `build.rs` - Copies GNOME extension to target dir during build (skipped on non-Linux)
- `flake.nix` - Nix packages + Home Manager module (Linux-only)

## Cargo Features

- `embed-gnome-extension` (default): Embeds GNOME extension in binary for `cargo install` support

## Current State

- [x] GNOME backend (DBus extension)
- [x] KDE backend (KWin script injection)
- [x] Wayland backend via `wlr-foreign-toplevel-management` (Sway, Hyprland, Niri)
- [x] Wayland backend via `cosmic-toplevel-info` (COSMIC)
- [x] Kanata reconnection on disconnect
- [x] macOS backend (NSWorkspace)
- [x] Windows backend (WinEvent hook)
- [ ] Testing on real environments (macOS, Windows)
- [ ] Config hot-reload

## Quick Test

```bash
# Requires kanata running: kanata -p 10000
cargo run -- -p 10000

# Or via nix
nix build && ./result/bin/kanata-switcher -p 10000
```

## Key Functions

| Function | File | Purpose |
|----------|------|---------|
| `detect_environment()` | `environ.rs` | Checks env vars to pick backend (Linux), compile-time for macOS/Windows |
| `platform::run()` | `platform/mod.rs` | Platform dispatch — calls linux/macos/windows entry point |
| `run_lifecycle_supervisor()` | `platform/linux.rs` | Supervises runtime target transitions and backend start/stop |
| `LifecycleProvider` | `lifecycle/mod.rs` | Selects `logind` continuous events or startup-only snapshot |
| `session_type_to_session_kind()` | `lifecycle/mod.rs` | Maps logind session type + active state to lifecycle domain |
| `resolve_runtime_target()` | `lifecycle/mod.rs` | Maps lifecycle state + desktop capabilities to concrete backend |
| `run_gnome()` | `backends/gnome.rs` | GNOME backend (DBus poll) |
| `run_kde()` | `backends/kde.rs` | KDE backend (KWin script) |
| `run_wayland()` | `backends/wayland.rs` | Unified Wayland backend (wlr/cosmic) |
| `run_x11()` | `backends/x11.rs` | X11 backend |
| `MacOsBackend::run()` | `backends/macos.rs` | macOS backend (NSWorkspace) |
| `WindowsBackend::run()` | `backends/windows.rs` | Windows backend (WinEvent hook) |
| `KanataClient` | `kanata.rs` | TCP client struct with reconnection |
| `match_rule()` | `focus_pipeline.rs` | Rule matching logic |
| `resolve_install_gnome_extension()` | `lifecycle/mod.rs` | CLI flag resolution (last wins) |
| `install_gnome_extension()` | `lifecycle/mod.rs` | Tries filesystem, falls back to embedded |

## Wayland Protocol Support

The daemon uses standard Wayland protocols instead of compositor-specific IPC:

1. **wlr-foreign-toplevel-management** - works on wlroots compositors (Sway, Hyprland, Niri, etc.)
2. **cosmic-toplevel-info** - works on COSMIC (requires cosmic-workspace protocol as dependency)

Both protocols expose `title`, `app_id`, and `activated` state. The daemon tries wlr first, falls back to cosmic.

## Platform-specific Backend Notes

- **macOS**: Uses `NSWorkspaceDidActivateApplicationNotification` via a CFRunLoop thread. The `CFRunLoopRef` is stored as `usize` to satisfy `Send`. `current_window_info()` calls `query_focus_for_env` directly (no DBus like Linux). Signal handler catches Ctrl+C (`SIGINT`).
- **Windows**: Uses `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` on a message-pump thread. Process info extracted with `GetWindowModuleFileName` + `GetWindowTextW`. Signal handler catches Ctrl+C (`SIGINT`).
- **Linux**: Full supervisor lifecycle with logind, SNI indicator, GNOME ext setup, DBus control. Signal handlers for `SIGTERM/SIGINT/SIGHUP`.

## Gotchas

- GNOME requires shell restart after extension install (Wayland: logout/login)
- KDE script uses different API for KDE 5 vs 6 (`clientActivated` vs `windowActivated`)
- Kanata must be running with `-p PORT` before daemon starts
- Default layer: auto-detected from Kanata unless `{"default": "layer"}` entry in config
- GNOME extension auto-install is default; use `--no-install-gnome-extension` to disable
- Continuous lifecycle transitions require login1 (`org.freedesktop.login1`); otherwise backend selection is startup-only
