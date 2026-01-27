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
- [ ] Daemon exits with clear error if no display env detected
