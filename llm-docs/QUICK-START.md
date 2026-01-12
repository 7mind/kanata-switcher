# Quick Start for LLM Successors

**IMPORTANT**: Do NOT add line numbers to this documentation. They are unimportant details that easily go out of sync with the actual code. Use function/struct names and `grep` to locate code.

## Project Summary

Kanata layer switcher daemon - switches keyboard layers based on focused window. Single Rust daemon supports GNOME, KDE, and Wayland compositors (Sway, Hyprland, Niri, COSMIC, etc.) via standard protocols.

## Key Files

- `src/daemon/main.rs` - Rust daemon (all backends)
- `src/gnome-extension/` - GNOME Shell extension (bundled, auto-installed)
- `src/protocols/` - Wayland protocol XMLs (cosmic-toplevel-info, cosmic-workspace)
- `build.rs` - Copies GNOME extension to target dir during build
- `flake.nix` - Nix packages + Home Manager module

## Cargo Features

- `embed-gnome-extension` (default): Embeds GNOME extension in binary for `cargo install` support

## Current State

- [x] GNOME backend (DBus extension)
- [x] KDE backend (KWin script injection)
- [x] Wayland backend via `wlr-foreign-toplevel-management` (Sway, Hyprland, Niri)
- [x] Wayland backend via `cosmic-toplevel-info` (COSMIC)
- [x] Kanata reconnection on disconnect
- [ ] Testing on real environments
- [ ] Config hot-reload

## Quick Test

```bash
# Requires kanata running: kanata -p 10000
cargo run -- -p 10000

# Or via nix
nix build && ./result/bin/kanata-switcher -p 10000
```

## Key Functions in main.rs

| Function | Purpose |
|----------|---------|
| `detect_environment()` | Checks env vars to pick backend |
| `run_gnome()` | GNOME backend (DBus poll) |
| `run_kde()` | KDE backend (KWin script) |
| `run_wayland()` | Unified Wayland backend (wlr/cosmic) |
| `KanataClient` | TCP client struct with reconnection |
| `match_rule()` | Rule matching logic |
| `resolve_install_gnome_extension()` | CLI flag resolution (last wins) |
| `install_gnome_extension()` | Tries filesystem, falls back to embedded |

## Wayland Protocol Support

The daemon uses standard Wayland protocols instead of compositor-specific IPC:

1. **wlr-foreign-toplevel-management** - works on wlroots compositors (Sway, Hyprland, Niri, etc.)
2. **cosmic-toplevel-info** - works on COSMIC (requires cosmic-workspace protocol as dependency)

Both protocols expose `title`, `app_id`, and `activated` state. The daemon tries wlr first, falls back to cosmic.

## Gotchas

- GNOME requires shell restart after extension install (Wayland: logout/login)
- KDE script uses different API for KDE 5 vs 6 (`clientActivated` vs `windowActivated`)
- Kanata must be running with `-p PORT` before daemon starts
- Default layer: auto-detected from Kanata unless `{"default": "layer"}` entry in config
- GNOME extension auto-install is default; use `--no-install-gnome-extension` to disable
