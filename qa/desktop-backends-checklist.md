# Desktop Backends Checklist

Last tested: YYYY-MM-DD
Environment: (distro, DE/compositor, session type)

## KDE Plasma
- [ ] KWin script loads automatically
- [ ] Focus changes trigger expected actions
- [ ] Daemon start applies current focused window without extra focus change
- [ ] Pause/unpause re-queries current focus (no cached focus)
- [ ] DBus backend stays connected

## wlroots (Sway/Hyprland/Niri)
- [ ] wlr-foreign-toplevel events received
- [ ] Focus changes trigger expected actions
- [ ] Daemon start applies current focused window without extra focus change
- [ ] Pause/unpause re-queries current focus (no cached focus)

## COSMIC
- [ ] cosmic-toplevel-info events received
- [ ] Focus changes trigger expected actions
- [ ] Daemon start applies current focused window without extra focus change
- [ ] Pause/unpause re-queries current focus (no cached focus)

## X11
- [ ] _NET_ACTIVE_WINDOW tracking works
- [ ] Focus changes trigger expected actions
- [ ] Daemon start applies current focused window without extra focus change
- [ ] Pause/unpause re-queries current focus (no cached focus)

## Unknown/unsupported
- [ ] Daemon exits with clear error if no display env detected
