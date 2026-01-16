The project daemon is located at `src/daemon/` (Rust).

# GNOME Shell support investigation
- [x] Inspect the pre-existing solutions for GNOME Shell (Wayland) for other remappers in ./local directory - keymapper, keyd, xremap . Write down the findings that will help you implement a similar extension for layer-switching with kanata in llm-docs md.

# Implementation
- [x] Create GNOME Shell extension implementing layer switching for kanata
- [x] Create KDE Plasma (KWin script) support
- [x] Unified Wayland backend via `wlr-foreign-toplevel-management` (Sway, Hyprland, Niri, etc.)
- [x] COSMIC support via `cosmic-toplevel-info` protocol (same unified backend)
- [x] Support virtual key actions (`(vk ...)` in kanata config)
- [ ] Support reload config actions (`(lrld)`, `(lrld-num N)`, `(lrpv)`, `(lrnx)`)
- [ ] Support ReloadFile action (TCP exclusive, no kanata syntax equivalent)

# Testing & Polish
- [x] Test GNOME Shell extension on actual GNOME session
- [x] Test KDE KWin script + daemon on actual KDE session
- [x] Test daemon on Sway/Hyprland/Niri
- [x] Test daemon on COSMIC
- [x] Add proper error handling and reconnection logic
- [ ] Add config file watching for hot-reload
- [ ] Package for distribution
