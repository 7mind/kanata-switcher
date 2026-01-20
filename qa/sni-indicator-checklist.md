# SNI Indicator Checklist (Non-GNOME)

Last tested: YYYY-MM-DD
Environment: (DE/compositor, panel/bar, session type)

## Indicator lifecycle
- [ ] SNI indicator appears by default on non-GNOME
- [ ] `--no-indicator` suppresses it
- [ ] Logs show SNI startup and watcher online/offline

## Visual behavior
- [ ] Layer glyph updates on focus changes
- [ ] VK glyph updates (single key / count / infinity)
- [ ] Layer glyph is white and VK glyph is cyan (matches GNOME indicator)
- [ ] Tooltip shows layer and virtual keys

## Menu actions
- [ ] Pause toggles pause state
- [ ] Unpause resumes focus processing
- [ ] "Show app layer only" toggles focus-only view
- [ ] Restart restarts daemon

## Persistence
- [ ] "Show app layer only" persists across daemon restarts when GSettings is available
- [ ] `--indicator-focus-only true|false` overrides startup value without locking the toggle

## Failure behavior
- [ ] If SNI cannot be started, daemon keeps running and logs error
