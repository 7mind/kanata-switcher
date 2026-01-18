# CLI Control Commands Checklist

Last tested: YYYY-MM-DD
Environment: (distro, DE/compositor, session type)

## Restart
- [ ] Start daemon normally
- [ ] Run `kanata-switcher --restart`
- [ ] Running daemon restarts (log shows restart)
- [ ] Caller exits cleanly

## Pause
- [ ] Run `kanata-switcher --pause`
- [ ] Daemon logs pause and stops reacting to focus
- [ ] Managed virtual keys released
- [ ] Layer resets to default

## Unpause
- [ ] Run `kanata-switcher --unpause`
- [ ] Daemon resumes focus processing
- [ ] Focus changes trigger expected actions
