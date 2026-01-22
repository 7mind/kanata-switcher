# Focus Rules Checklist

Last tested: 2026-01-22
Environment: GNOME, Wayland

## Rule matching
- [x] Class match triggers expected layer change
- [ ] Title match overrides class when configured
- [ ] Regex patterns behave as expected
- [x] Unfocus switches to default layer

## Fallthrough behavior
- [ ] Non-fallthrough stops further rules
- [x] Fallthrough executes all matching rules
- [ ] Layer changes are applied in rule order

## Virtual keys (managed)
- [x] Pressed on focus
- [x] Released on unfocus
- [ ] Released when rule no longer matches
- [ ] Pausing releases managed keys

## Raw virtual key actions
- [ ] Press/Release/Tap/Toggle actions are sent
- [ ] Raw actions coexist with layer changes

## Source tracking
- [x] Focus-based layer updates show as focus source
- [x] External layer changes still surface in indicator
