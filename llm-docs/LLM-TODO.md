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
- 2026-03-13: Logind provider startup now requires successful non-empty `Type` for active sessions; initialization no longer defaults `Type` to empty on read errors (prevents silent idle `NoSession` startup).
- 2026-03-13: Logind lifecycle listener now fails fast at process level on signal/decode errors (`exit(1)` from the detached monitor task) so lifecycle supervision cannot silently continue in a stale state.
- 2026-03-12: Supervisor no longer exits on transient Wayland capability resolver failures; it logs probe errors, keeps current backend running, and retries on later recheck/snapshot events.
- 2026-03-12: SNI guard is now created whenever indicator is enabled (independent of startup GNOME), so runtime transitions can still enable indicator/local controls after non-graphical or GNOME startup states.
- 2026-03-12: Runtime SNI transition planning restarts indicator resources on every environment change (including same-mode transitions like X11 <-> Wayland) to mirror restart-equivalent DE transition semantics.
- 2026-03-12: Wayland desktop-capability backend selection (GNOME/KDE vs generic) is now periodically re-evaluated from the last lifecycle snapshot, so startup races no longer get stuck on generic Wayland until next logind `Active`/`Type` change.
- 2026-03-12: SNI indicator lifecycle is now runtime-environment-driven; control mode is rebuilt on backend-environment transitions (including `Unknown` -> graphical), instead of being fixed at process startup.
- 2026-03-11: SNI local control now reads runtime environment from lifecycle-supervised backend state (not startup detection), so unpause focus refresh tracks backend switches (e.g. wayland -> tty -> x11).
- 2026-03-11: Lifecycle supervisor now waits on backend completion signals in `tokio::select!` (including after provider exhaustion), so unexpected backend exits/restarts are observed immediately instead of stalling idle.
- 2026-03-11: Runtime lifecycle supervision refactor landed. Daemon now uses login1-driven continuous backend transitions (with startup-snapshot fallback when login1 is unavailable), and the Home Manager graphical-session restart helper service was removed from flake outputs.
- 2026-01-18: logind session monitoring failure is non-fatal; daemon continues without native terminal switching.
- 2026-01-18: logind session resolution now falls back to the user's `Display` session when `GetSessionByPID` reports no session (systemd user service with lingering).
- 2026-01-19: logind object path parsing accepts signatures `o`, `s`, `v`, or structures containing an object path (robust reply decoding).
- 2026-01-20: SNI indicator icon colors aligned with GNOME (layer white, VK cyan).
- 2026-01-20: SNI indicator glyphs use Noto Sans Mono bitmap size 32; VK overflow renders as "9+".
- 2026-01-23: Legacy kanata support - versions without `RequestFakeKeyNames` API are detected via error response, reconnect skips the request. VK validation is bypassed for legacy kanata (all VKs pass through).
- 2026-01-23: Config Rule struct uses `#[serde(deny_unknown_fields)]` to reject typos like `native_terminal` (should be `on_native_terminal`).
- 2026-01-23: Rules without class/title matchers require `fallthrough: true` (otherwise would match everything and stop further matching).
- 2026-01-27: DBus control service starts on Wayland/X11 regardless of SNI status so Pause/Unpause/Restart commands still work if SNI fails.
