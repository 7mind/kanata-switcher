The project daemon is located at `src/daemon/` (Rust).

# GNOME Shell support investigation
- [x] Inspect the pre-existing solutions for GNOME Shell (Wayland) for other remappers in ./local directory - keymapper, keyd, xremap . Write down the findings that will help you implement a similar extension for layer-switching with kanata in llm-docs md.

# Implementation
- [x] Create GNOME Shell extension implementing layer switching for kanata
- [x] Create KDE Plasma (KWin script) support
- [x] Unified Wayland backend via `wlr-foreign-toplevel-management` (Sway, Hyprland, Niri, etc.)
- [x] COSMIC support via `cosmic-toplevel-info` protocol (same unified backend)
- [x] Support virtual key actions (`(vk ...)` in kanata config)
- [x] Handle native terminal switching via logind `on_native_terminal` rule
- [ ] Support reload config actions (`(lrld)`, `(lrld-num N)`, `(lrpv)`, `(lrnx)`)
- [ ] Support ReloadFile action (TCP exclusive, no kanata syntax equivalent)

# Testing & Polish
- [x] Test GNOME Shell extension on actual GNOME session
- [x] Test KDE KWin script + daemon on actual KDE session
- [x] Test daemon on Sway/Hyprland/Niri
- [x] Test daemon on COSMIC
- [x] Add proper error handling and reconnection logic
- [x] Add automated tests for rule matching and VK lifecycle (`src/daemon/tests.rs`)
- [x] Add integration tests for DE backends (`src/daemon/integration_tests.rs`)
- [x] GNOME prefs load via gnome-extensions-app (ExtensionPreferences import path)
- [x] Add GJS test for GNOME top bar character formatting (Nix check)
- [x] GNOME indicator updates on focus-based layer changes with focus-only toggle
- [x] Add DBus GetStatus test for focus source
- [x] Persist SNI focus-only setting via GSettings with CLI override
- [ ] Add config file watching for hot-reload
- [ ] Package for distribution

# Code Quality
- [x] Unify GNOME/KDE into shared DBus backend (`DbusWindowFocusService`)
- [x] FocusAction as algebraic data type with ordered action list
- [x] Fallthrough executes ALL matching actions (layers, VKs, raw actions)
- [x] `nix run .#test` runs tests via cargo-nextest (compilation cached, execution fresh)
- [x] X11 tests use hardcoded display numbers for parallel nextest execution

# Notes
- 2026-01-18: logind session monitoring failure is non-fatal; daemon continues without native terminal switching.
- 2026-01-18: logind session resolution now falls back to the user’s `Display` session when `GetSessionByPID` reports no session (systemd user service with lingering).
- 2026-01-19: logind object path parsing accepts object paths, single-field structures, or strings (robust reply decoding).
