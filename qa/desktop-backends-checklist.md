# Desktop Backends Checklist

Last tested: YYYY-MM-DD
Environment: (distro, DE/compositor, session type)

## KDE Plasma
- [ ] KWin script loads automatically
- [ ] Focus changes trigger expected actions
- [ ] DBus backend stays connected

## wlroots (Sway/Hyprland/Niri)
- [ ] wlr-foreign-toplevel events received
- [ ] Focus changes trigger expected actions

## COSMIC
- [ ] cosmic-toplevel-info events received
- [ ] Focus changes trigger expected actions

## X11
- [ ] _NET_ACTIVE_WINDOW tracking works
- [ ] Focus changes trigger expected actions

## Unknown/unsupported
- [ ] Daemon exits with clear error if no display env detected
