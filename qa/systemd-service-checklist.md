# Systemd Service Checklist

Last tested: YYYY-MM-DD
Environment: (distro, DE/compositor, session type)

## User service setup
- [ ] `systemd/kanata-switcher.service` copied to user unit dir
- [ ] `systemctl --user daemon-reload` succeeds
- [ ] `systemctl --user enable --now kanata-switcher` starts service

## Logging
- [ ] `journalctl --user -u kanata-switcher` shows startup logs
- [ ] `--quiet-focus` reduces focus spam

## Shutdown
- [ ] SIGTERM/SIGINT switches to default layer and exits
- [ ] Service restarts cleanly
