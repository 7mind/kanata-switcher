# Nix Module Keyboards Multiplex Checklist

- [ ] Last tested: YYYY-MM-DD on distro/DE:

Preconditions:
- [ ] Kanata instances are running and listening on distinct ports.
- [ ] `services.kanata-switcher.enable = true` is set.
- [ ] `services.kanata-switcher.keyboards` contains at least two entries with different `kanataPort` values.

Checks:
- [ ] Rebuild and switch the system/home config.
- [ ] `systemctl --user status kanata-switcher-<keyboard>.service` succeeds for each configured keyboard.
- [ ] `systemctl --user status kanata-switcher.service` is absent in keyboards mode.
- [ ] Each instance connects to its expected port (verify logs show the configured port).
- [ ] Focus switching works per instance using each instance's rules.
- [ ] GNOME extension integration still works when `gnomeExtension.enable = true`.

Negative checks:
- [ ] Setting both `keyboards` and top-level `kanataPort`/`kanataHost`/`configFile`/`settings`/`logging` fails evaluation with the module assertion.
- [ ] `keyboards.<name>.configFile` and `keyboards.<name>.settings` together fail evaluation with the module assertion.
