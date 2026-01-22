# Systemd Service Checklist

Last tested: 2026-01-22
Environment: NixOS, GNOME, Wayland

## User service setup
- [ ] `systemd/kanata-switcher.service` copied to user unit dir
- [x] `systemctl --user daemon-reload` succeeds
- [x] `systemctl --user enable --now kanata-switcher` starts service

## Logging
- [x] `journalctl --user -u kanata-switcher` shows startup logs
- [x] `--quiet-focus` reduces focus spam

## Shutdown
- [x] SIGTERM/SIGINT switches to default layer and exits
- [x] Service restarts cleanly
