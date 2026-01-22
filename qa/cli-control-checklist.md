# CLI Control Commands Checklist

Last tested: 2026-01-22
Environment: NixOS, KDE, Wayland

## Restart
- [x] Start daemon normally
- [x] Run `kanata-switcher --restart`
- [x] Running daemon restarts (log shows restart)
- [x] Caller exits cleanly

## Pause
- [x] Run `kanata-switcher --pause`
- [x] Daemon logs pause and stops reacting to focus
- [x] Managed virtual keys released
- [x] Layer resets to default

## Unpause
- [x] Run `kanata-switcher --unpause`
- [x] Daemon resumes focus processing
- [x] Focus changes trigger expected actions
