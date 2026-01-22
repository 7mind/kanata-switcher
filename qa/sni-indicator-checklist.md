# SNI Indicator Checklist (Non-GNOME)

Last tested: 2026-01-22
Environment: KDE, KDE Panel, Wayland

## Indicator lifecycle
- [x] SNI indicator appears by default on non-GNOME
- [x] `--no-indicator` suppresses it
- [x] Logs show SNI startup and watcher online/offline
- [ ] Logs show SNI watcher offline

## Visual behavior
- [x] Layer glyph updates on focus changes
- [x] VK glyph updates (single key / count / overflow)
- [x] Layer glyph is white and VK glyph is cyan (matches GNOME indicator)
- [ ] Glyphs use Noto Sans Mono bitmap (size 32) and VK overflow shows "9+"
- [x] Tooltip shows layer and virtual keys

## Menu actions
- [x] Pause toggles pause state
- [x] Unpause resumes focus processing
- [x] "Show app layer only" toggles focus-only view
- [x] Restart restarts daemon

## Persistence
- [x] "Show app layer only" persists across daemon restarts when GSettings is available
- [ ] "Show app layer only" persists across daemon restarts when daemon is launched via systemd unit
- [x] `--indicator-focus-only true|false` overrides startup value without locking the toggle

## Failure behavior
- [ ] If SNI cannot be started, daemon keeps running and logs error
