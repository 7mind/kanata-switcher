# Systemd Service Checklist

Last tested: 2026-01-22
Environment: NixOS, GNOME, Wayland

## User service setup
- [x] `systemctl --user daemon-reload` succeeds
- [x] `systemctl --user enable --now kanata-switcher` starts service

## Logging
- [x] `journalctl --user -u kanata-switcher` shows startup logs
- [x] `--quiet-focus` reduces focus spam

## Shutdown
- [x] SIGTERM/SIGINT switches to default layer and exits
- [x] Service restarts cleanly

## Persistent lifecycle (v1.2.0)

Not yet manually verified. Requires systemd-logind/elogind, Kanata, a config with an `on_native_terminal` rule, and a user manager that remains running across logout (for example, with lingering already enabled).

- [ ] Start the user service before graphical login; it remains alive while idle and begins focus-based switching after login without a manual restart.
- [ ] Record `systemctl --user show kanata-switcher -p MainPID -p NRestarts`, log out and back in, and confirm both values are unchanged and focus-based switching resumes.
- [ ] Switch to a Linux console with Ctrl+Alt+F*, confirm the native-terminal rule applies, and return to the desktop; the current focused window's rule is restored.
- [ ] Log into another supported desktop or change between X11 and Wayland; confirm backend selection and display endpoints refresh without restarting the daemon.
- [ ] Run `kanata-switcher --pause` and `kanata-switcher --unpause` during idle and after login; control requests succeed whenever the session bus is available.
