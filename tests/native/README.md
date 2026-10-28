# Native desktop integration

These tests launch the release daemon and three real desktop applications. Real
NSWorkspace notifications (macOS) or WinEvent hooks (Windows) pass through the
production backend, config parser and rule engine to a TCP Kanata protocol peer.
The peer substitutes for Kanata; keyboard devices/drivers are not exercised.
The existing Rust platform pipeline tests use synthetic `WindowInfo` values and
complement this suite; they do not establish native event delivery.

The suite checks initial focus before daemon startup, A → B → A activation,
virtual-key release/layer/press order, unmatched-app default fallback, quiet
steady state, and service-style shutdown while a virtual key is held. Windows
also checks title changes on the same foreground HWND. The scenario runs twice.
Rust tests additionally exercise watcher start/stop in the same process,
pre-requested shutdown, event-source failure and pause/unpause focus refresh.

## Run

Use a logged-in, unlocked desktop with no concurrent focus-changing applications:

```sh
cargo test --locked
cargo build --locked --release
# macOS: Xcode command-line tools (swiftc and swift)
python3 tests/native/run.py --binary target/release/kanata-switcher
# Windows: Developer PowerShell/Command Prompt for Visual Studio (cl.exe)
python tests/native/run.py --binary target/release/kanata-switcher.exe
```

macOS fixtures are temporary `.app` bundles with distinct bundle identifiers.
Windows fixtures are separate native Win32 processes with executable names that
also exercise case/space normalization. No downloaded desktop apps, AppleScript,
Accessibility permission grants, fabricated focus notifications, or skip-on-CI
paths are used. Missing desktop/focus access fails the tests. Windows foreground
permission is explicitly transferred to the target helper. macOS reports bundle
IDs only; it does not report window titles. Windows reports normalized executable
stems and native window titles.

The `Rust and native desktop integration` workflow runs Linux, macOS and Windows
Cargo tests and release builds, then executes this suite on the native macOS and
Windows GitHub-hosted desktops. Nix/module checks remain in the existing CI.
`target/native-test-logs/` contains daemon/config/fixture logs, uploaded on success
or failure; the failure output also includes the daemon log. Each operation has a
bounded timeout and child processes are cleaned up on failure.
