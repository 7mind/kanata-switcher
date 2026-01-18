# Focus Rules Checklist

Last tested: YYYY-MM-DD
Environment: (DE/compositor, session type)

## Rule matching
- [ ] Class match triggers expected layer change
- [ ] Title match overrides class when configured
- [ ] Regex patterns behave as expected
- [ ] Unfocus switches to default layer

## Fallthrough behavior
- [ ] Non-fallthrough stops further rules
- [ ] Fallthrough executes all matching rules
- [ ] Layer changes are applied in rule order

## Virtual keys (managed)
- [ ] Pressed on focus
- [ ] Released on unfocus
- [ ] Released when rule no longer matches
- [ ] Pausing releases managed keys

## Raw virtual key actions
- [ ] Press/Release/Tap/Toggle actions are sent
- [ ] Raw actions coexist with layer changes

## Source tracking
- [ ] Focus-based layer updates show as focus source
- [ ] External layer changes still surface in indicator
