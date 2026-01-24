# GNOME Extension Checklist

Last tested: 2026-01-24
Environment: NixOS, GNOME 49, Wayland

## Top bar indicator
- [x] Indicator appears when enabled
- [x] Layer letter updates on focus changes
- [x] Virtual key indicator updates (single key / count)
- [ ] Virtual key indicator updates (infinity symbol on 9+ VKs)
- [x] Indicator hides when disabled in prefs
- [x] Indicator shows '?' when daemon terminates (stop service while indicator visible)
- [x] Daemon restart while extension is already running updates the layer without changing focus (no manual focus change needed)

## Menu actions
- [x] Pause toggle reflects daemon state on startup for a fresh daemon (switch to a native terminal, export `XDG_CURRENT_DESKTOP=GNOME`, `XDG_RUNTIME_DIR=/run/user/$(id -u)`, and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`, start daemon, restart GNOME Shell, enable extension, open menu and confirm it shows unpaused without toggling)
- [ ] Pause toggle reflects daemon state on startup for a pre-paused daemon (switch to a native terminal, export `XDG_CURRENT_DESKTOP=GNOME`, `XDG_RUNTIME_DIR=/run/user/$(id -u)`, and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`, start daemon, run `kanata-switcher --pause`, restart GNOME Shell, enable extension, open menu and confirm it shows paused without toggling)
- [x] Pause toggle pauses daemon and updates indicator state
- [x] Unpause resumes processing
- [x] Settings opens extension preferences
- [x] Restart triggers daemon restart

## Preferences
- [x] "Show top bar icon" toggles indicator
- [x] "Show app layer only" toggles focus-only view
- [x] Preferences load in gnome-extensions-app
