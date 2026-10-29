# Desktop Backends Checklist

Last tested: 2026-01-27
Environment: NixOS, COSMIC, Wayland

## KDE Plasma
- [x] KWin script loads automatically
- [x] Focus changes trigger expected actions
- [x] Daemon start applies current focused window without extra focus change
- [x] Pause/unpause re-queries current focus (no cached focus)
- [x] DBus backend stays connected

## wlroots (Sway/Hyprland/Niri)
- [x] wlr-foreign-toplevel events received
- [x] Focus changes trigger expected actions
- [x] Daemon start applies current focused window without extra focus change
- [x] Pause/unpause re-queries current focus (no cached focus)

## COSMIC
- [x] cosmic-toplevel-info events received
- [x] Focus changes trigger expected actions
- [x] Daemon start applies current focused window without extra focus change
- [x] Pause/unpause re-queries current focus (no cached focus)

## X11
- [ ] _NET_ACTIVE_WINDOW tracking works
- [ ] Focus changes trigger expected actions
- [ ] Daemon start applies current focused window without extra focus change
- [ ] Pause/unpause re-queries current focus (no cached focus)

## Unknown/unsupported
- [x] Daemon exits with clear error if no display env detected

## Native macOS and Windows

Automated setup and scope: `tests/native/README.md`.

- [ ] Run native desktop integration on an unlocked macOS session.
- [ ] Run native desktop integration from a Windows MSVC developer shell.
- [ ] With real Kanata, verify bundle-ID rules on macOS and executable/title rules on Windows.
- [ ] While holding a managed virtual key, terminate the daemon (SIGTERM on macOS, Ctrl+C/Break on Windows); verify key release and default-layer reset.
- [ ] Confirm no focus actions fire after shutdown; restart and switch apps again.
- [ ] Verify Windows title rules update without switching away from the foreground window.

macOS window titles are unavailable through NSWorkspace; title-based macOS
rules cannot be exercised by native focus notifications.
