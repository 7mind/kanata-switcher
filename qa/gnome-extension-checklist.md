# GNOME Extension Checklist

Last tested: YYYY-MM-DD
Environment: (GNOME version, session type)

## Top bar indicator
- [ ] Indicator appears when enabled
- [ ] Layer letter updates on focus changes
- [ ] Virtual key indicator updates (single key / count / infinity)
- [ ] Indicator hides when disabled in prefs
- [ ] Indicator shows '?' when daemon terminates (stop service while indicator visible)
- [ ] Daemon restart while extension is already running updates the layer without changing focus (no manual focus change needed)

## Menu actions
- [ ] Pause toggle reflects daemon state on startup for a fresh daemon (switch to a native terminal, export `XDG_CURRENT_DESKTOP=GNOME`, `XDG_RUNTIME_DIR=/run/user/$(id -u)`, and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`, start daemon, restart GNOME Shell, enable extension, open menu and confirm it shows unpaused without toggling)
- [ ] Pause toggle reflects daemon state on startup for a pre-paused daemon (switch to a native terminal, export `XDG_CURRENT_DESKTOP=GNOME`, `XDG_RUNTIME_DIR=/run/user/$(id -u)`, and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`, start daemon, run `kanata-switcher --pause`, restart GNOME Shell, enable extension, open menu and confirm it shows paused without toggling)
- [ ] Pause toggle pauses daemon and updates indicator state
- [ ] Unpause resumes processing
- [ ] Settings opens extension preferences
- [ ] Restart triggers daemon restart

## Preferences
- [ ] "Show top bar icon" toggles indicator
- [ ] "Show app layer only" toggles focus-only view
- [ ] Preferences load in gnome-extensions-app
