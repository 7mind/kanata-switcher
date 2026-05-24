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
- [x] macOS `cargo check` passes (0 errors, 0 warnings)
- [ ] Add macOS tests (normalize_process_name tests exist for windows)
- [ ] Config file watching for hot-reload

# Notes
- 2026-05-23: Cross-platform support landed. macOS/Windows backends implement `FocusBackend` via NSWorkspace notifications (macOS) and WinEvent foreground hooks (Windows). Platform dispatch at `src/daemon/platform/`. All Linux-specific code gated behind `#[cfg(target_os = "linux")]`.
- 2026-05-18: Fixed native-terminal lifecycle regression during GNOME/Wayland <-> Linux console VT switches. Real logind events for the monitored graphical display session report `Active=false` while `Type` remains `wayland`; lifecycle decoding now maps inactive graphical sessions to `SessionKind::NativeTerminal` instead of `NoSession`, so supervisor transitions `gnome -> linux-console` and fires `on_native_terminal`. Explicit display-clear snapshots (`User.Display=/`) still emit `NoSession`. Added regressions `test_decode_logind_change_maps_inactive_graphical_session_to_native_terminal` and `test_run_lifecycle_supervisor_inactive_graphical_session_enters_linux_console`.
- 2026-05-13: Fixed KDE session-start and shutdown regressions from field log. Continuous logind supervision now treats generic Wayland backend start failure during a Wayland graphical session as transient: it stays alive in Idle and lets capability rechecks promote to KDE/GNOME once the DE bus owner appears. KWin script cleanup is now bounded (250ms per stop/unload call) and logs external cleanup failures instead of panicking from `Drop`, covering one-shot focus-query `UnknownObject` cleanup failures and SIGTERM hangs while unloading scripts. Added regressions `test_run_lifecycle_supervisor_retries_after_transient_generic_wayland_start_failure`, `test_run_kde_tolerates_kwin_cleanup_unknown_object`, and `test_run_kde_bounds_kwin_cleanup_latency`.
- 2026-05-11: SNI tray menu (non-GNOME) gained a "Quit" item. Quit shuts the daemon down via `ShutdownHandle::request()` directly (same effect as SIGTERM/SIGINT) — no DBus `Quit` method is added, since the SNI runs in-process with the daemon for both Local and Dbus control modes. `SniLocalControl` and `SniDbusControl` both carry the daemon's `ShutdownHandle`. Added unit tests `test_sni_local_control_quit_triggers_shutdown_handle` and extended `test_sni_menu_actions_dispatch_control` to assert Quit dispatch.
- 2026-05-11: DBus namespace is now partitioned into `com.github.kanata.Switcher.instances.<suffix>` (daemon bus names, always multiplexed) and `com.github.kanata.Switcher.extensions.<de>` (interface + path for DE bridges; the GNOME extension piggybacks on `org.gnome.Shell` and does not own a bus name in our namespace). Each daemon registers a unique well-known name derived from `--dbus-suffix` or auto-derived from host/port (`p10000` for defaults). The control CLI broadcasts to every `instances.*` owner by default and unicasts when `--dbus-suffix` is given. GNOME extension is now multi-indicator (one indicator per daemon, panel label layer/VK only, keyboard name in tooltip) and emits `FocusChanged` signals (one emitter, N daemon subscribers) instead of calling `WindowFocus` per-daemon. KDE injects per-daemon KWin scripts that push to the per-instance bus name. The Nix `keyboards.<name>` mode passes `--dbus-suffix <name>` per instance. Added unit tests for suffix helpers, broadcast/unicast routing, GNOME signal subscription, KDE script targeting, persistent reconnect with non-default suffix; GJS tests for new helper module; `nixos-module-keyboard-suffix-check` Nix-eval check; QA checklist `qa/dbus-multiplex-checklist.md`.
