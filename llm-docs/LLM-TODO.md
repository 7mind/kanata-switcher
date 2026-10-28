The project daemon is located at `src/daemon/` (Rust). The repo now tracks upstream `7mind/kanata-switcher` branch `persistent-daemon`.

# Cross-platform support
- [x] Rebased onto upstream/persistent-daemon (FocusBackend trait, modular directory structure)
- [x] macOS backend: `backends/macos.rs` implements `FocusBackend` via NSWorkspace notifications
- [x] Windows backend: `backends/windows.rs` implements `FocusBackend` via WinEvent hook
- [x] Platform dispatch: `platform/` module with `linux.rs`, `macos.rs`, `windows.rs`
- [x] Cargo.toml: platform-conditional dependencies for mac/windows
- [x] `Environment` and `BackendKind` enums: MacOS, Windows variants
- [x] `detect_environment()`: compile-time dispatch for mac/windows
- [ ] Test on actual macOS (requires kanata running)
- [ ] Test on actual Windows (requires kanata running)
- [ ] Move untracked dev tools to repo (kanata-vk-agent/, window_tools/)

# Linux-only features (upstream)
- GNOME Shell extension, KDE KWin script, Wayland, X11 backends
- DBus control, SNI indicator, logind lifecycle supervisor
- NixOS/Home Manager module

# Implementation
- [x] MacOsBackend: `FocusBackend` impl via NSWorkspaceDidActivateApplicationNotification
- [x] WindowsBackend: `FocusBackend` impl via EVENT_SYSTEM_FOREGROUND SetWinEventHook
- [x] platform/mod.rs: common setup + single-cfg dispatch
- [x] platform/linux.rs: full Linux daemon (supervisor, sni, gnome, dbus)
- [x] platform/macos.rs: signal handling + MacOsBackend
- [x] platform/windows.rs: signal handling + WindowsBackend
- [ ] Package for distribution (mac: .app bundle, win: installer)

# Testing & Polish
- [x] Native desktop suite (`tests/native/run.py`): real fixture apps → OS notifications/hooks → release daemon → TCP peer; strict action order and graceful shutdown. CI matrix covers macOS, Windows and Linux. Native desktop results must be read from CI; cross-compilation alone is insufficient.
- [x] Shared native loop tests cover pause/unpause resync, key release, pre-requested shutdown and source failure; platform watcher tests exercise repeated immediate stop/start.
- [x] macOS `cargo check` passes (0 errors, 0 warnings)
- [x] macOS synthetic pipeline tests: `integration_tests/macos.rs` — 8 tests covering bundle ID rule matching, regex, title matching, full async pipeline with MockKanataServer, pause/unpause cycle, and `current_window_info()` smoke test
- [x] Windows synthetic pipeline tests: `integration_tests/windows.rs` — 7 tests covering process name rule matching, multiple focus changes, title matching, normalize pipeline, full async pipeline, and pause/unpause cycle
- [x] `integration_tests` module enabled on macOS/Windows (was Linux-only); common module Linux-only submodules gated; mock server `set_nonblocking(false)` fix for macOS accept() inheritance
- [ ] Config file watching for hot-reload

# Notes
- 2026-10-27: Native macOS CI exposed cached frontmostApplication lag inside activation delivery. The observer now stores the NSWorkspace notification payload before waking the shared loop, retaining the latest snapshot while coalescing. Windows native tests pass on windows-2022. Native scenarios now repeat five times to exercise activation timing.
- 2026-10-27: First true macOS CI run exposed missing notification delivery with only a background CFRunLoop. macOS now pumps the actual process main run loop while Tokio runs the daemon task; native fixture focus acknowledgements distinguish activation failure from missing backend events.
- 2026-10-27: Native watchers now register before querying focus, coalesce fresh queries, own/join listener threads, and release managed keys on shutdown. Windows watches foreground titles; macOS handles SIGTERM; Windows handles Ctrl+Break. Verified 271 local Cargo tests, release build, five GJS suites and cross-target macOS/Windows Clippy. Native runtime assertions are wired into CI and await their platform runs. Luna reviewed original bugs and changes.
- 2026-10-27: Restored Linux compilation and lifecycle factory tests after platform extraction; Linux control/autostart dispatch runs before config/connection, covered by subprocess + private DBus tests. Xvfb fixtures disable reset between readiness probe and test clients. Cargo tests/release and all five GJS suites pass; local Nix execution remains unavailable (cache policy and missing sandbox shell).
- 2026-10-27: Merged current upstream persistent-daemon; preserved both ignore lists and release notes. Baseline Cargo tests/build expose pre-existing PR compile failures; repairs and native integration coverage follow. Nix validation attempted but cache.nixos.org is denied by this execution environment.
- 2026-05-23: Cross-platform support landed. macOS/Windows backends implement `FocusBackend` via NSWorkspace notifications (macOS) and WinEvent foreground hooks (Windows). Platform dispatch at `src/daemon/platform/`. All Linux-specific code gated behind `#[cfg(target_os = "linux")]`.
- 2026-05-24: Release automation split package-version bumps from dependency updates. `devops/release.sh` updates only the root `kanata-switcher` lock entry; `devops/update-libraries.sh` updates dependency lock entries separately.
- 2026-05-18: Fixed native-terminal lifecycle regression during GNOME/Wayland <-> Linux console VT switches. Real logind events for the monitored graphical display session report `Active=false` while `Type` remains `wayland`; lifecycle decoding now maps inactive graphical sessions to `SessionKind::NativeTerminal` instead of `NoSession`, so supervisor transitions `gnome -> linux-console` and fires `on_native_terminal`. Explicit display-clear snapshots (`User.Display=/`) still emit `NoSession`. Added regressions `test_decode_logind_change_maps_inactive_graphical_session_to_native_terminal` and `test_run_lifecycle_supervisor_inactive_graphical_session_enters_linux_console`.
- 2026-05-13: Fixed KDE session-start and shutdown regressions from field log. Continuous logind supervision now treats generic Wayland backend start failure during a Wayland graphical session as transient: it stays alive in Idle and lets capability rechecks promote to KDE/GNOME once the DE bus owner appears. KWin script cleanup is now bounded (250ms per stop/unload call) and logs external cleanup failures instead of panicking from `Drop`, covering one-shot focus-query `UnknownObject` cleanup failures and SIGTERM hangs while unloading scripts. Added regressions `test_run_lifecycle_supervisor_retries_after_transient_generic_wayland_start_failure`, `test_run_kde_tolerates_kwin_cleanup_unknown_object`, and `test_run_kde_bounds_kwin_cleanup_latency`.
- 2026-05-11: SNI tray menu (non-GNOME) gained a "Quit" item. Quit shuts the daemon down via `ShutdownHandle::request()` directly (same effect as SIGTERM/SIGINT) — no DBus `Quit` method is added, since the SNI runs in-process with the daemon for both Local and Dbus control modes. `SniLocalControl` and `SniDbusControl` both carry the daemon's `ShutdownHandle`. Added unit tests `test_sni_local_control_quit_triggers_shutdown_handle` and extended `test_sni_menu_actions_dispatch_control` to assert Quit dispatch.
- 2026-05-11: DBus namespace is now partitioned into `com.github.kanata.Switcher.instances.<suffix>` (daemon bus names, always multiplexed) and `com.github.kanata.Switcher.extensions.<de>` (interface + path for DE bridges; the GNOME extension piggybacks on `org.gnome.Shell` and does not own a bus name in our namespace). Each daemon registers a unique well-known name derived from `--dbus-suffix` or auto-derived from host/port (`p10000` for defaults). The control CLI broadcasts to every `instances.*` owner by default and unicasts when `--dbus-suffix` is given. GNOME extension is now multi-indicator (one indicator per daemon, panel label layer/VK only, keyboard name in tooltip) and emits `FocusChanged` signals (one emitter, N daemon subscribers) instead of calling `WindowFocus` per-daemon. KDE injects per-daemon KWin scripts that push to the per-instance bus name. The Nix `keyboards.<name>` mode passes `--dbus-suffix <name>` per instance. Added unit tests for suffix helpers, broadcast/unicast routing, GNOME signal subscription, KDE script targeting, persistent reconnect with non-default suffix; GJS tests for new helper module; `nixos-module-keyboard-suffix-check` Nix-eval check; QA checklist `qa/dbus-multiplex-checklist.md`.
