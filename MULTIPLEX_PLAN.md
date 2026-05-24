# kanata-switcher Multi-Instance Plan

## Document purpose

Implementation plan for allowing multiple `kanata-switcher` daemons to coexist on a
single session bus, one per keyboard. Today every daemon competes for the well-known
DBus name `com.github.kanata.Switcher`; only one can own it, so the rest lose the
control API and the GNOME/KDE focus push channel.

The change has three coordinated parts:

1. Daemon: register a **per-instance** well-known name, derived from `--dbus-suffix`
   (or auto-derived from host/port). Control CLI honors the same suffix.
2. GNOME extension: enumerate daemons on the bus, present **one indicator per
   daemon**, and emit focus updates as a **signal** (one emitter, N subscribers)
   instead of calling each daemon individually.
3. KDE: each daemon injects its own KWin script that calls back on its per-instance
   name. KWin already supports many concurrent scripts; the existing per-script UUID
   path keeps them isolated.

Wayland, X11 and COSMIC backends are unaffected — they read focus directly from the
compositor/server, so each daemon already has a private channel.

## Goals (acceptance criteria)

1. Two daemons started together on the same session bus both register a unique
   well-known name and remain registered.
2. `kanata-switcher --dbus-suffix kinesis --pause` targets only the daemon registered
   as `com.github.kanata.Switcher.kinesis`.
3. The Nix `keyboards.<name>` mode passes `--dbus-suffix <name>` for each instance
   without further user configuration.
4. On GNOME with two daemons running, the top bar shows two indicators, one per
   keyboard; each reflects its own daemon's layer/VK state and routes its menu
   actions to the matching daemon. The panel label carries **no** keyboard-name
   prefix; the keyboard name appears only in the tooltip.
5. On KDE with two daemons running, each daemon independently observes focus events
   via its own KWin script.
6. Single-instance without `--dbus-suffix` parameter should use an auto-derived multiplexed name. It is a non-goal to support extension/daemon interoperation with mismatching versions.
7. New behavior is covered by Rust unit tests, Rust integration tests (mock dbus
   session bus where needed), and GJS tests for the extension. `cargo test`,
   `nix run .#test`, `nix build` and `nix flake check` all pass.
8. CLI commands given without a `--dbus-suffix` must enumerate and send commands to all daemons via dbus (broadcast), if `--dbus-suffix` is present, only to that daemon (unicast)

## Non-goals

- No persisted per-indicator settings yet (focus-only toggle stays global).
- No KDE coordinator-relay election; each daemon injects its own script. Trade-off:
  on KDE the focus path is N × DBus calls per focus change, which is acceptable for
  N ≤ a few keyboards. If this is ever measured to matter, revisit.

## Domain model changes

### DBus namespace partitioning

The project's DBus name surface splits into two cleanly separated subtrees,
applied consistently across **bus names, interface names, and object paths**:

- `com.github.kanata.Switcher.instances.<suffix>` — used as a daemon's
  well-known **bus name** (one per daemon instance). Daemon discovery
  (extension, control CLI) filters by this prefix.
- `com.github.kanata.Switcher.extensions.<de>` — used as **interface name**
  and **object path** for desktop-environment bridges. Currently the only
  inhabitant is `com.github.kanata.Switcher.extensions.GNOME` exposed by the
  GNOME Shell extension. The extension does **not** claim a separate
  well-known bus name; like today, it `export`s its object onto the session
  bus under GNOME Shell's existing connection name `org.gnome.Shell` (see
  `extension.js` — `_focusDbus.export(Gio.DBus.session, FOCUS_DBUS_PATH)` and
  no `own_name` call). The daemon addresses the extension by routing to
  `org.gnome.Shell` with the renamed interface + path. KDE has no bridge —
  KWin scripts `callDBus` into the daemon's bus name directly.

Because the extension owns no bus name in the project's namespace, the
daemon-discovery filter is unambiguous: every `com.github.kanata.Switcher.*`
**bus name** is a daemon, and the filter is exactly
`starts_with("com.github.kanata.Switcher.instances.")` with no exceptions.

Object paths follow the same shape:

- daemon control: `/com/github/kanata/Switcher` (per-connection — no collision)
- GNOME bridge: `/com/github/kanata/Switcher/extensions/GNOME`
- KDE one-shot focus query: `/com/github/kanata/Switcher/KdeQuery<N>` (per-connection)

### DBus name resolution

Add pure functions in `src/daemon/main.rs`:

```rust
/// Always returns a non-empty sanitized suffix. Default host + default port
/// yields `"p10000"`.
fn derive_default_dbus_suffix(host: &str, port: u16) -> String;

/// Sanitize a suffix to DBus name element rules: `[A-Za-z0-9_-]`,
/// must not start with a digit, must not be empty after sanitization,
/// length capped to a sensible value (e.g. 64 chars).
fn sanitize_dbus_suffix(raw: &str) -> Result<String, DbusSuffixError>;

/// Resolve CLI flag + defaults into the effective suffix. Always succeeds
/// with a non-empty suffix (or returns `Err` if the CLI value is invalid).
fn resolve_dbus_suffix(cli: Option<&str>, host: &str, port: u16) -> Result<String, DbusSuffixError>;

/// Compose the per-instance well-known name:
/// `com.github.kanata.Switcher.instances.<suffix>`.
fn effective_dbus_name(suffix: &str) -> String;
```

Derivation rules:

- CLI `--dbus-suffix VAL` (after sanitization) wins.
- Otherwise, if `host == "127.0.0.1"`: suffix `p<port>` (e.g. `p22334`).
- Otherwise: suffix `h<sanitized_host>_p<port>` (e.g. `h192_168_1_2_p22334`).

Sanitization rules for `sanitize_dbus_suffix`:

- Replace any char not in `[A-Za-z0-9_-]` with `_`.
- If the first character is a digit, prepend `_` (DBus name elements cannot start
  with a digit).
- Reject empty input.
- Cap at 64 chars.

The constants `DBUS_NAME` and `DBUS_INTERFACE` become per-instance variables.
`DBUS_BASE_NAME` = `"com.github.kanata.Switcher.instances"` is the namespace root
for daemons. The interface attribute on `DbusWindowFocusService` stays a literal
(`com.github.kanata.Switcher` — interface names do not need to vary with bus
names; zbus's `#[interface]` macro requires a literal). The object path
(`/com/github/kanata/Switcher`) is constant — paths are per-connection.

### Focus push model (extension → daemons)

Today: GNOME extension calls `WindowFocus(class, title)` method on the daemon's
well-known name; KWin script does the same.

New:

- GNOME extension emits `FocusChanged(class, title)` signal from its exported
  object at path `/com/github/kanata/Switcher/extensions/GNOME`, interface
  `com.github.kanata.Switcher.extensions.GNOME`. Sender bus name is
  `org.gnome.Shell` (the extension piggybacks on Shell, not its own name).
  Daemons running the GNOME backend subscribe via a match rule
  (`type=signal, sender=org.gnome.Shell, interface=com.github.kanata.Switcher.extensions.GNOME, member=FocusChanged, path=/com/github/kanata/Switcher/extensions/GNOME`).
  Signal model is the only viable shape here: one emitter, N receivers
  (multiplex method-call pull would be N calls per focus change, scaling
  focus latency with keyboard count).
- KDE: per-daemon KWin script (each daemon injects its own UUID-scoped script).
  The script `callDBus`-es `WindowFocus(class, title)` on its own daemon's
  per-instance name. Already isolated by daemon-owned script path.

The `WindowFocus` method handler stays on the daemon (still used by KDE).
The extension-side method call is removed.

### Focus pull model (daemon → extension/DE), unpause path

Synchronous in every backend. No protocol change for KDE; GNOME stays
synchronous too via a method call into the renamed extension bus name.

| Backend | Mechanism (unchanged in shape) |
|---|---|
| X11 | Direct `GetProperty` on `_NET_ACTIVE_WINDOW` against own X connection |
| Wayland (wlr) | Direct toplevel walk on own Wayland connection |
| Wayland (COSMIC) | Direct cosmic-toplevel-info walk on own Wayland connection |
| KDE | Inject one-shot KWin script via `org.kde.KWin.loadScript`; script calls `Focus(class, title)` back into the daemon's per-instance bus name; daemon awaits oneshot channel with 5s timeout. Async at protocol level only because KWin scripts cannot register their own bus name. |
| GNOME | Synchronous method call to `org.gnome.Shell` at path `/com/github/kanata/Switcher/extensions/GNOME` with interface `com.github.kanata.Switcher.extensions.GNOME`, method `GetFocus` (interface + path renamed from `com.github.kanata.Switcher.Gnome` / `/com/github/kanata/Switcher/Gnome`; the bus-name target is unchanged — the extension still piggybacks on `org.gnome.Shell`). |

Rationale for keeping GNOME pull synchronous: GNOME Shell extensions have full
DBus permissions and can export callable objects on the session bus, so the
daemon can issue a one-shot method call and get an inline reply. KDE has to be
async because KWin JS scripts cannot register or export their own DBus
services — the daemon has to inject a script that pushes the answer back via
`callDBus`. There's no benefit to making GNOME match KDE's async shape when it
doesn't have to.

### Indicator model (GNOME extension)

The extension owns a `Map<busName, IndicatorEntry>`. Each entry carries:

- One `PanelMenu.Button` (indicator) with its layer/VK labels.
- One `Gio.DBusProxy` for the daemon (its per-instance name).
- Subscriptions for that daemon's `StatusChanged` and `PausedChanged` signals.
- A cached last-status used by the existing `selectStatus` logic.

Discovery and lifecycle:

- On `enable()`: call `org.freedesktop.DBus.ListNames`, filter to names
  starting with `com.github.kanata.Switcher.instances.`, create one entry each.
- Subscribe to `NameOwnerChanged` with
  `arg0namespace=com.github.kanata.Switcher.instances` to track daemons
  appearing or disappearing at runtime. Create/destroy entries accordingly.
- On `disable()`: tear down every entry.

UI rules (per user instruction):

- Panel label has no keyboard-name prefix. Each indicator shows only that
  daemon's layer letter and VK glyph (existing `formatLayerLetter` /
  `formatVirtualKeys`).
- Tooltip text includes the keyboard name. Derivation: keyboard name = the
  substring after `com.github.kanata.Switcher.instances.` — always non-empty
  because daemons always multiplex.
- Settings remain global: `show-top-bar-icon` and `show-focus-layer-only` apply
  to every indicator.

### Daemon discovery for control CLI

`kanata-switcher --restart|--pause|--unpause [--dbus-suffix SUFFIX]`:

- If `--dbus-suffix` is given (explicit), call the method on
  `com.github.kanata.Switcher.instances.<suffix>` only (unicast).
- Otherwise, enumerate `com.github.kanata.Switcher.instances.*` via
  `ListNames`, call the method on each (broadcast). Print per-daemon result.
  Empty enumeration is an error ("no daemons running").
- Targeted call to an unknown suffix surfaces a clear error message including
  the resolved name.

### Nix module wiring

In `flake.nix`, `buildExecArgs` adds `--dbus-suffix <keyboardName>` when
keyboard mode is active. Single-instance mode adds no `--dbus-suffix`; the
daemon auto-derives `p10000` (default host + default port).

## File-by-file change list

### `src/daemon/main.rs`

- Replace `const DBUS_NAME` / `const DBUS_INTERFACE` with:
  - `DBUS_BASE_NAME = "com.github.kanata.Switcher.instances"` — namespace
    root for daemon bus names.
  - `DBUS_BASE_INTERFACE = "com.github.kanata.Switcher"` — control interface
    name (unchanged from today; interfaces don't collide across distinct
    bus-name owners).
  - `GNOME_FOCUS_OBJECT_PATH = "/com/github/kanata/Switcher/extensions/GNOME"`
    and `GNOME_FOCUS_INTERFACE = "com.github.kanata.Switcher.extensions.GNOME"`
    (renamed from `/com/github/kanata/Switcher/Gnome` and
    `com.github.kanata.Switcher.Gnome`). The bus-name target stays
    `GNOME_SHELL_BUS_NAME = "org.gnome.Shell"` — the extension does not own
    a separate bus name.
- Add pure helpers:
  - `derive_default_dbus_suffix(host: &str, port: u16) -> String` — always
    non-empty. Default host + default port yields `"p10000"`.
  - `sanitize_dbus_suffix(raw: &str) -> Result<String, DbusSuffixError>`.
  - `resolve_dbus_suffix(cli: Option<&str>, host: &str, port: u16) -> Result<String, DbusSuffixError>` —
    explicit CLI value (after sanitization) wins; otherwise derived.
  - `effective_dbus_name(suffix: &str) -> String` — returns
    `format!("{DBUS_BASE_NAME}.{suffix}")` =
    `"com.github.kanata.Switcher.instances.<suffix>"`.
  - `is_daemon_bus_name(name: &str) -> bool` — `name.starts_with("com.github.kanata.Switcher.instances.")`.
    No exceptions: the `extensions.*` and `instances.*` subtrees never
    overlap.
- Extend `Args` with `dbus_suffix: Option<String>` (`--dbus-suffix`). Empty
  string rejected via clap value parser that delegates to `sanitize_dbus_suffix`.
- `send_control_command` / `send_control_command_with_connection`: take an
  explicit name parameter; no fallback to a hardcoded constant.
- Add `enumerate_daemon_names(connection: &Connection) -> Result<Vec<String>, DynError>`:
  call `ListNames` and filter via `is_daemon_bus_name`. No special-case
  exclusions needed.
- Add `send_control_command_broadcast(connection, command) -> Result<BroadcastReport, DynError>`:
  enumerates daemons and calls the method on each with a short per-call
  timeout. Returns a per-name success/error report. Empty enumeration is an
  error ("no daemons running").
- Control CLI flow: if `--dbus-suffix` is present, unicast to the resolved
  name. If absent, broadcast.
- `register_dbus_service_with_runtime_environment` and
  `run_persistent_dbus_service_with_connector`: take effective name as a
  parameter and use it for `request_name`, `receive_name_lost_with_args`,
  and error messages. The interface attribute on `DbusWindowFocusService`
  stays literal (`com.github.kanata.Switcher`) because zbus's `#[interface]`
  macro requires a literal — interface names don't conflict across
  distinct bus-name owners, so this is correct.
- `start_persistent_dbus_service` / `start_persistent_dbus_service_with_connector`:
  add effective-name parameter.
- `run_gnome`:
  - GNOME backend keeps the **synchronous** `GetFocus` pull at startup /
    unpause. Bus-name target stays `org.gnome.Shell`; interface and path
    renamed to the `…extensions.GNOME` namespace.
  - For live focus events, drop the dependence on a `WindowFocus` method
    call into the daemon. Subscribe to a `FocusChanged` signal via a
    `zbus::MatchRule` with
    `type=signal, sender=org.gnome.Shell,
    interface=com.github.kanata.Switcher.extensions.GNOME,
    member=FocusChanged,
    path=/com/github/kanata/Switcher/extensions/GNOME`.
  - Rationale: pull stays sync (one method call, inline reply, no timeout
    needed); push is a signal because the extension fans out to N daemons
    — an N-call method-fanout would scale focus latency with keyboard
    count.
- `run_kde`: KWin script `callDBus` targets the per-instance bus name. The
  control interface stays `com.github.kanata.Switcher`. UUID-scoped script
  paths already isolate scripts across daemons. KDE pull-on-unpause logic
  (`query_kde_focus`) is unchanged in shape — already async at the protocol
  level because KWin scripts can't own a bus name; the registered query
  object lives at `/com/github/kanata/Switcher/KdeQuery<N>` on the daemon's
  per-instance bus name.
- `DbusWindowFocusService::window_focus` method handler retained — still
  called by KDE script callbacks.
- `autostart_passthrough_args`: add `dbus_suffix` to
  `AUTOSTART_PASSTHROUGH_OPTIONS`; emit `--dbus-suffix VAL` in the match.

### `src/gnome-extension/extension.js`

- Rename DBus constants:
  - `DBUS_NAME` (the global daemon name) → drop (no single daemon name
    any more; each `IndicatorEntry` carries its own bus-name string).
  - `FOCUS_DBUS_PATH` → `/com/github/kanata/Switcher/extensions/GNOME`
  - `FOCUS_DBUS_INTERFACE` → `com.github.kanata.Switcher.extensions.GNOME`
  - The extension does **not** call `own_name`. It continues to `export`
    its `_focusDbus` object on the session bus under GNOME Shell's
    existing connection (`org.gnome.Shell`), the same way it does today.
    Only the path + interface change.
- Replace top-level `_daemonProxy`/`_indicator`/`_status`/`_focusStatus` with
  a `Map<busName, IndicatorEntry>`. Each `IndicatorEntry` owns:
  - one `PanelMenu.Button` with its layer + VK labels and pause/restart menu,
  - one `Gio.DBusProxy` bound to that daemon's bus name,
  - subscriptions for that daemon's `StatusChanged` and `PausedChanged`,
  - cached last-status / last-focus-status / paused flag.
- Discovery:
  - On `enable()`: call `org.freedesktop.DBus.ListNames` on the session bus,
    filter via `filterDaemonNames` (= prefix
    `com.github.kanata.Switcher.instances.`), create one entry per match.
  - Subscribe to `NameOwnerChanged` with
    `arg0namespace=com.github.kanata.Switcher.instances`. On owner-added
    events whose name passes the filter, create an entry; on owner-removed
    events, destroy the entry.
  - Subscriptions and entry teardown happen in `disable()`.
- Focus push: keep the existing `_notifyFocus()` triggering on
  `notify::focus-window`, but instead of calling each daemon's
  `WindowFocus` method, emit a `FocusChanged` signal on the extension's
  own exported object:
  - Extend `FOCUS_DBUS_XML` to declare
    `<signal name="FocusChanged"><arg type="s"/><arg type="s"/></signal>`
    alongside the existing `GetFocus` method.
  - Emit via
    `this._focusDbus.emit_signal('FocusChanged', new GLib.Variant('(ss)', [class, title]))`.
  - The `GetFocus` method handler stays (used by daemon's sync unpause
    pull).
- Tooltip wiring: each indicator's tooltip composes from the keyboard name
  parsed from its bus name plus the layer / VKs lines. Panel label keeps
  the existing layer-letter + VK-glyph layout with **no** keyboard prefix.
- Global settings (`show-top-bar-icon`, `show-focus-layer-only`): when a
  setting changes, iterate over the map and apply.

### `src/gnome-extension/extension-multiplex.js` (new helper module)

Pure helpers extracted for testability:

- `parseKeyboardName(busName)` — returns the substring after
  `com.github.kanata.Switcher.instances.`. Returns `""` for any other
  input (defensive — daemon discovery already filters by this prefix).
- `composeTooltip({keyboard, layer, virtualKeys})` — joins non-empty lines
  with `\n`. Keyboard line goes first when present.
- `filterDaemonNames(busNames)` — keep names starting with
  `com.github.kanata.Switcher.instances.`. No further exclusions; the
  `extensions.*` subtree is disjoint by construction.

### `flake.nix`

- `buildExecArgs`: append `--dbus-suffix <name>` when `keyboardMode` is true,
  using the keyboard attrset key as the suffix value. In single-instance mode
  no `--dbus-suffix` is added; the daemon auto-derives from default host/port
  to `p10000`.
- Add a new check `nixos-module-keyboard-suffix-check`: evaluates the NixOS
  module with two example keyboards and asserts that each generated user
  unit's `ExecStart` includes `--dbus-suffix <keyboardName>`.

### `qa/dbus-multiplex-checklist.md` (new)

Human checklist covering: two daemons co-running on session bus; both
indicators on GNOME; per-indicator Pause/Restart; tooltips show keyboard
names; broadcast control CLI pauses both daemons; targeted control CLI pauses
only the named one; KDE multi-script focus delivery.

### Docs

- `README.md` LLM section: document `--dbus-suffix`, the always-suffixed
  default name derivation, the broadcast-by-default control CLI semantics,
  and the per-keyboard indicator behavior.
- `llm-docs/architecture.md`: update the DBus section — per-instance bus
  names always, single shared control interface, signal-based GNOME focus
  push, control-CLI broadcast vs unicast.
- `llm-docs/implementation-notes.md`: add decision entries for the
  always-multiplex rule, signal-vs-method split, no-prefix indicator label
  rule, and control-CLI broadcast default.
- `llm-docs/LLM-TODO.md`: dated entry summarizing the change.

## Test plan (mandatory — all entries below to be implemented)

### A. Pure Rust unit tests — `src/daemon/tests.rs`

- `derive_default_dbus_suffix`:
  - default host (`127.0.0.1`) + default port (`10000`) → `"p10000"`.
  - default host + port `22334` → `"p22334"`.
  - host `192.168.1.2` + port `22334` → `"h192_168_1_2_p22334"`.
  - host with dots, dashes, and uppercase letters → sanitized correctly.
  - IPv6-style host (e.g. `::1`) → colons become underscores and the result
    starts with a letter (`h_1_p…` or similar — pinned by the test).
- `sanitize_dbus_suffix`:
  - alphanumeric input → unchanged.
  - input with `.`, `:`, `-`, space → underscores (`-` is allowed by DBus
    name element rules; this test pins the chosen behavior, which the plan
    sets to "replace with underscore" for predictability).
  - input starting with digit → leading underscore added.
  - empty input → `Err`.
  - input longer than 64 chars → either truncated to 64 or `Err` (test pins
    the implementation choice; recommended: `Err` so users notice).
- `resolve_dbus_suffix`:
  - explicit CLI value, sanitization required → returns sanitized explicit
    value (auto-derivation is skipped).
  - explicit CLI value, empty after sanitization → `Err`.
  - no CLI value, default host/port → `"p10000"`.
  - no CLI value, non-default port → `"p22334"`.
- `effective_dbus_name("p10000")` → `"com.github.kanata.Switcher.instances.p10000"`.
- `effective_dbus_name("kinesis")` → `"com.github.kanata.Switcher.instances.kinesis"`.
- `is_daemon_bus_name`:
  - keeps `com.github.kanata.Switcher.instances.p10000`,
    `com.github.kanata.Switcher.instances.kinesis`.
  - rejects `com.github.kanata.Switcher.extensions.GNOME`,
    `com.github.kanata.Switcher`, `com.github.kanata.Switcher.instances`
    (bare prefix without trailing component), and arbitrary unrelated
    names.
- `autostart_passthrough_args` regression: `--dbus-suffix foo` round-trips
  through autostart args.

### B. Integration tests — `src/daemon/integration_tests.rs`

All use the existing mock session-bus harness (zbus + dbus-daemon launched
from tests).

- `test_two_daemons_register_independent_names`:
  - Start daemons with `--dbus-suffix a` and `--dbus-suffix b`. Verify
    `name_has_owner("com.github.kanata.Switcher.instances.a")` and
    `…instances.b` are true; `com.github.kanata.Switcher` and
    `com.github.kanata.Switcher.instances` (bare prefix) are not owned.
- `test_default_suffix_used_when_flag_absent`:
  - Single daemon started without `--dbus-suffix`, default host/port → owns
    `com.github.kanata.Switcher.instances.p10000`.
- `test_default_suffix_derived_from_non_default_port`:
  - Daemon with `-p 22334`, no `--dbus-suffix` → owns
    `com.github.kanata.Switcher.instances.p22334`.
- `test_control_command_targets_specific_daemon_when_suffix_given`:
  - Two daemons (`a`, `b`); send `Pause` with `--dbus-suffix a`; assert
    daemon `a` reports paused, daemon `b` does not.
- `test_control_command_broadcasts_when_suffix_absent`:
  - Two daemons with distinct suffixes; CLI sends `Pause` without
    `--dbus-suffix`; assert both daemons report paused.
- `test_control_command_broadcast_with_no_daemons_errors`:
  - Send `Pause` with no daemons running → CLI surfaces a clear "no daemons
    found" error.
- `test_control_command_targeted_unknown_suffix_errors`:
  - `--dbus-suffix nope` with no matching owner → error referencing the
    resolved name.
- `test_control_command_broadcast_partial_failure_reports_per_daemon`:
  - Two daemons; one is stalled / unresponsive; broadcast completes within
    a bounded time and reports per-daemon outcome.
- `test_control_command_broadcast_ignores_unrelated_namespace_names`:
  - Stand up a fake owner of, say, `com.github.kanata.Switcher.foo` (a
    name in the namespace prefix but not under `instances.`); confirm
    `enumerate_daemon_names` does **not** include it.
- `test_gnome_daemon_subscribes_to_focus_signal`:
  - Mock extension-side service (registered under the mock bus's existing
    connection, no separate bus-name ownership needed) emits a
    `FocusChanged("firefox", "")` signal from the renamed path/interface;
    daemon (with a rule matching `firefox` class) issues a `ChangeLayer`
    to the mock Kanata server. The MatchRule's `sender=` is the mock
    GNOME-Shell-stand-in name used by the test harness.
- `test_gnome_daemon_multiple_instances_receive_focus_signal`:
  - One mock emitter, two daemons subscribed; emit one signal; both
    daemons issue layer-change commands to their respective mock Kanata
    servers.
- `test_gnome_daemon_unpause_pull_uses_renamed_interface_and_path`:
  - Mock service exposes `GetFocus` at
    `/com/github/kanata/Switcher/extensions/GNOME` with interface
    `com.github.kanata.Switcher.extensions.GNOME`; daemon performs the
    sync pull on backend start / unpause and applies the returned focus.
    Asserts the call lands at the renamed path/interface (bus-name
    routing target is the mock's GNOME-Shell-stand-in name).
- `test_kde_script_targets_per_instance_name`:
  - Spin up the mock KWin scripting interface and capture the script body
    the daemon writes; assert the script's `callDBus` references
    `com.github.kanata.Switcher.instances.<suffix>` (not the base prefix).
- `test_kde_two_daemons_inject_independent_scripts`:
  - Two daemons enter the KDE backend (mocked KWin); each injects its own
    UUID-scoped script targeting its own bus name; KWin "fires" a focus
    event on both; both daemons handle it independently.
- `test_persistent_dbus_service_reconnect_uses_effective_name`:
  - Mirror existing reconnect/NameLost regressions with a non-default
    suffix to ensure the rename plumbing stays consistent through the
    persistent service loop.

### C. GJS extension tests — `tests/`

Existing pattern: `KANATA_SWITCHER_SRC=… gjs -m tests/…` driven from
`flake.nix`'s `gnome-format` check.

- `tests/gnome-extension-multiplex.js` (new):
  - `parseKeyboardName("com.github.kanata.Switcher.instances.kinesis")` →
    `"kinesis"`. `parseKeyboardName("com.github.kanata.Switcher.extensions.GNOME")`
    → `""` (extensions subtree never matches).
  - `composeTooltip` formats correctly: keyboard + layer + VKs lines,
    omitting empty lines; keyboard-only and layer-only forms work.
  - `filterDaemonNames` keeps
    `com.github.kanata.Switcher.instances.p10000`,
    `com.github.kanata.Switcher.instances.kinesis`; rejects
    `com.github.kanata.Switcher.extensions.GNOME`, the bare
    `com.github.kanata.Switcher.instances` prefix, and unrelated names.
  - Map-based registry behavior: simulated `NameOwnerChanged` events add
    and remove entries; re-adding an already-known name updates the
    proxy and does not duplicate the widget.
  - Panel-label rule: the indicator's label string equals
    `formatLayerLetter(layer) + formatVirtualKeys(vks)` and contains **no**
    keyboard name, even with two entries present.
  - Tooltip rule: tooltip text contains the keyboard name on each entry.
- `tests/gnome-extension-focus.js`:
  - Add regression: focus push goes via `emit_signal('FocusChanged', …)`
    on the extension's own exported object at
    `/com/github/kanata/Switcher/extensions/GNOME` with `(ss)` params; the
    extension never calls `Gio.DBus.session.call` for focus pushes.
  - Add regression: the extension's `GetFocus` method remains exposed at
    the renamed bus name + path and still returns `(class, title)` synchronously.

### D. Nix-level check — `flake.nix`

- `nixos-module-keyboard-suffix-check`: evaluates the NixOS module with two
  example keyboards (`kinesis`, `framework13`); inspects the generated
  systemd user-unit text and asserts each contains the matching
  `--dbus-suffix <name>` argument.
- Existing `nixos-module-build` check stays.

### E. QA checklist — `qa/dbus-multiplex-checklist.md`

Manual scenarios:

1. Two daemons running on the same session: both indicators appear on the
   GNOME top bar; tooltips show correct keyboard names; panel labels show
   layer/VK only (no keyboard prefix).
2. Per-indicator Pause via menu: only its indicator shows paused; the other
   indicator still responds to focus events.
3. Per-indicator Restart: indicator briefly disappears and reappears.
4. `kanata-switcher --dbus-suffix kinesis --pause` pauses only the matching
   daemon.
5. `kanata-switcher --pause` (no suffix) pauses **both** daemons; CLI prints
   per-daemon result.
6. KDE: two daemons on KDE Plasma; both observe focus and switch their
   respective kanata layers.
7. Single-keyboard config (no `keyboards` block, default flake settings):
   daemon registers `com.github.kanata.Switcher.instances.p10000`; one
   indicator appears with tooltip keyboard line `p10000`.

## Execution order

1. Implement and unit-test the suffix derivation / sanitization / resolution
   helpers and the daemon-side `filter_daemon_names` helper.
2. Thread the effective name through daemon DBus registration paths,
   persistent service loop, KWin script generation. Add integration tests:
   independent name registration, default suffix derivation, KDE per-script
   targeting, persistent reconnect with non-default suffix.
3. Rename the GNOME extension's bus name + interface + object path to the
   `…extensions.GNOME` namespace on both sides (daemon constants and
   extension). Keep `GetFocus` as a sync method call (pull on backend
   start / unpause). Replace daemon-side `WindowFocus` method dependence
   with a `FocusChanged` signal subscription for live events. Add
   integration tests for both the sync pull and the signal-driven push.
4. Add `enumerate_daemon_names` + `send_control_command_broadcast`. Rework
   the control CLI flow to broadcast by default and unicast when
   `--dbus-suffix` is given. Add integration tests for broadcast, targeted,
   unknown-suffix, and no-daemons cases.
5. Rewrite the GNOME extension to the multi-indicator model with
   signal-based focus emission. Add GJS tests.
6. Update Nix module to pass `--dbus-suffix <keyboardName>` per instance.
   Add `nixos-module-keyboard-suffix-check`.
7. Documentation: README LLM section, `llm-docs/architecture.md`,
   `llm-docs/implementation-notes.md`, `llm-docs/LLM-TODO.md`, QA
   checklist.
8. Run `cargo test`, `cargo build --release`, `nix run .#test`, `nix build`,
   `nix flake check`. Commit per project guidelines (one shippable chunk
   per commit; tests land with the code).

Steps 2 + 3 + 4 + 5 should land in a single user-visible commit window (or
one merged sequence) because the daemon's per-instance name + extension
multi-indicator + control CLI broadcast are interdependent — they cannot be
shipped piecemeal without breaking the extension/daemon contract. The
implementation can still be reviewed step by step in the source tree.

## Risks and mitigations

- **Risk:** `NameOwnerChanged` storms (daemon flapping) cause indicator
  thrashing in GNOME.
  - Mitigation: indicator entry creation is idempotent — re-adding for an
    already known name updates the proxy reference and does not duplicate
    the widget. Covered by a GJS test.
- **Risk:** `arg0namespace` match-rule filter unsupported on older GLib
  versions used by some GNOME Shell builds.
  - Mitigation: target GNOME 45+ (current supported baseline already uses
    modern GLib); if needed, fall back to wildcard subscription with
    client-side filtering. Decide during implementation by checking the
    minimum GNOME version we support.
- **Risk:** broadcast control CLI on a busy session bus enumerates many
  unrelated names; a buggy third-party owner of a
  `com.github.kanata.Switcher.instances.<X>` name could intercept calls.
  - Mitigation: the `com.github.kanata.Switcher.instances.*` subtree is
    project-specific by convention; only this project's daemons should
    register there. The disjoint conceptual `extensions.*` subtree is for
    interface names / object paths only — the extension does not own a
    bus name in our prefix, so it cannot be enumerated as a daemon.
- **Risk:** broadcast control CLI's per-daemon failure reporting becomes
  noisy when daemons are unresponsive.
  - Mitigation: use a short per-call timeout and report errors compactly
    (one line per daemon); test covers partial failure.
- **Risk:** zbus interface attribute is a literal — per-instance interface
  names would require more invasive rework.
  - Mitigation: interface names don't need to vary with bus names (they
    don't conflict across distinct owners). Keep the interface literal
    (`com.github.kanata.Switcher` for the control interface,
    `com.github.kanata.Switcher.extensions.GNOME` for the extension
    interface); multiplex only the bus name.
