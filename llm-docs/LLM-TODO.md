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
- 2026-03-17: Persistent DBus service reconnect loop now applies bounded retry backoff (max 2s) for **all** setup failures, including `DBusProxy::new` and `NameLost` subscription setup errors. This prevents busy-loop/log-flood behavior when name-loss monitoring setup fails under transient broker issues. Added regressions `test_dbus_reconnect_delay_caps_at_two_seconds` and `test_wait_for_dbus_reconnect_retry_backoffs_for_name_lost_monitor_setup_errors`.
- 2026-03-16: Continuous lifecycle mode now applies **conditional** Wayland resolver-error fallback: if a Wayland snapshot fails to resolve and current target is non-Wayland-family (`idle`/`linux-console`/`x11`), supervisor falls back to generic Wayland; if current target is already Wayland-family (`gnome`/`kde`/`wayland`), supervisor keeps the active backend and retries on rechecks. Added regressions `test_run_lifecycle_supervisor_continuous_wayland_resolver_error_falls_back_to_generic_wayland` and `test_run_lifecycle_supervisor_continuous_wayland_resolver_error_keeps_active_gnome_backend`.
- 2026-03-16: Startup-snapshot lifecycle now preserves explicit GNOME/KDE identity from startup env detection (`session_type` hint `gnome`/`kde`) and resolves those targets directly in snapshot mode instead of depending on one-shot session-bus owner probing. This prevents no-logind startup races from pinning GNOME/KDE sessions to generic Wayland for the whole daemon run. Added regressions for GNOME/KDE startup snapshot target resolution.
- 2026-03-16: KDE backend startup/runtime-mode probing now gates on KWin `/Scripting` interface readiness (`org.kde.kwin.Scripting` export via introspection) and retries probe attempts before failing. This prevents transient startup races (KWin name owned before scripting API export) from aborting backend transition. Added integration regression `test_run_kde_waits_for_scripting_interface_before_runtime_probe`.
- 2026-03-16: Wayland logind display override handling now validates that `Session.Display` is a plausible Wayland socket value before using it; X11-style values like `:0` are ignored so startup/unpause/backend start falls back to normal env/default Wayland connection instead of failing. Startup-snapshot lifecycle no longer exits on initial Wayland resolver/probe errors; it falls back to generic Wayland target for that one-shot snapshot.
- 2026-03-16: Wayland desktop flavor resolution now selects GNOME by GNOME Shell bus ownership (not extension-focus readiness probe). This restores no-logind startup behavior so startup-snapshot sessions do not get pinned to generic Wayland when the extension is missing/initializing at process start; GNOME setup then runs on the initial GNOME backend transition. Added regressions for startup-snapshot GNOME selection and startup-supervisor GNOME setup in startup-only mode.
- 2026-03-16: Local SNI unpause is now bound to the control creation context (environment + focus-query context), instead of sampling `runtime_environment.current()` at click time. This removes the X11/Wayland -> GNOME/KDE transition race where stale Local control could pass GNOME/KDE with no session connection and panic during unpause focus refresh. Added regression coverage for this race.
- 2026-03-16: `LogindLifecycleProvider::new` now validates login1 lifecycle-monitor prerequisites before selecting the continuous provider. If login1 monitor init prerequisites fail (for example, `org.freedesktop.login1` missing or denied even though system bus is reachable), provider selection falls back to startup-snapshot mode instead of returning logind and later exiting from detached monitor init failure. Added regression tests for provider build fallback/success selection.
- 2026-03-16: Refactored X11/Wayland focus-query endpoint resolution to reuse the same generalized logind display-override resolver used by backend startup. Unpause/startup focus refresh no longer hardcodes env-only display resolution; added integration regressions for stale env on unpause (`test_x11_unpause_focus_query_uses_runtime_display_override`, `test_wayland_unpause_focus_query_uses_runtime_display_override`).
- 2026-03-16: Fixed KDE integration-test flake caused by cross-process `/tmp` script filename collisions. KWin runtime/query/probe script paths now include UUID suffixes (`run_kde`, `query_kde_focus`, runtime query-mode probe), and added unit regressions in `src/daemon/tests.rs` to lock UUID-scoped path generation.
- 2026-03-15: Runtime backend startup now refreshes X11/Wayland display endpoints from the active logind display session (`Session.Display`) and uses explicit connection endpoints (`x11rb::connect(Some(...))`, Wayland `Connection::from_socket`) instead of relying only on startup process env. This covers logout/login transitions where `DISPLAY`/`WAYLAND_DISPLAY` changed without daemon restart.
- 2026-03-15: Added integration regressions for stale graphical env vars with explicit runtime overrides: Wayland connection with stale `WAYLAND_DISPLAY` and X11 connection with stale `DISPLAY`.
- 2026-03-15: GNOME extension setup is now bound to runtime backend transitions into GNOME (not startup env detection only). This covers persistent-daemon flows like `Unknown -> GNOME` after login; added lifecycle regression test for non-GNOME startup transitioning into GNOME.
- 2026-03-15: Runtime-managed SNI now retries control initialization on transient start failures (timer + env-change wake) instead of waiting only on environment changes; added whitebox regression test that fails first start then verifies retry-based recovery.
- 2026-03-15: KDE backend startup (`run_kde`) now resolves KDE5/KDE6 API mode from the current runtime KWin session (DBus probe) instead of startup `KDE_SESSION_VERSION`; added integration regression test covering stale startup env with KDE6 runtime.
- 2026-03-14: Runtime-managed SNI transitions now bind status/pause/menu watcher task lifetimes to the active indicator handle. On Stop/Restart/drop, old watcher tasks are aborted with indicator shutdown, preventing unbounded task accumulation across `idle <-> x11/wayland/kde` transitions; added whitebox regression test.
- 2026-03-14: KDE unpause runtime mode resolution no longer depends on startup `KDE_SESSION_VERSION`. DBus unpause now probes KWin script object-path layout at runtime (`/Scripting/ScriptN` vs `/N`) and selects KDE6/KDE5 query mode accordingly; added regression test for mismatched startup env (`KDE_SESSION_VERSION=5`) with KDE6 runtime.
- 2026-03-14: DBus service registration is now daemon-persistent and lifecycle-independent. A dedicated manager keeps `com.github.kanata.Switcher` owned whenever session bus exists, handles `NameLost` push events, and reconnects/re-registers after bus loss. This prevents `--restart`/control API loss during idle logout/login transitions.
- 2026-03-14: Logind lifecycle monitor now detaches stale session properties monitoring when `User.Display` becomes `/`, so normal logout session teardown cannot kill the daemon; monitor waits for next display session path and reattaches.
- 2026-03-14: Logind lifecycle monitor now tracks `User.Display` transitions during runtime and reattaches to new display session paths after logout/login, preventing post-login stale idle supervision.
- 2026-03-14: Logind lifecycle provider initialization is now non-blocking. When pre-login display session is unavailable, the detached lifecycle monitor waits on login1 `User.Display` properties-changed signals (push-based) and attaches once ready, so daemon startup does not block and does not require external restart supervision.
- 2026-03-14: Design direction documented for pre-login persistence: non-blocking lifecycle init, immediate Idle supervision, push-based logind session readiness/events, and no dependence on external restart supervisors.
- 2026-03-14: Startup-snapshot lifecycle now fails hard when initial runtime target resolution errors (instead of skipping and staying idle with no backend).
- 2026-03-13: Logind properties-changed stream termination now fails fast (process exit) instead of silently exhausting the provider channel and freezing lifecycle transitions.
- 2026-03-13: Signal decode path now enforces the same active-session non-empty `Type` invariant as startup init; active snapshots with empty `Type` are rejected/fail-fast.
- 2026-03-13: Wayland capability polling is now disabled in startup-snapshot lifecycle mode; periodic re-resolve runs only for continuous (logind) providers.
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
