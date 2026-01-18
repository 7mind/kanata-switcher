# Autostart Checklist

## Preconditions
- Built `kanata-switcher` binary available in a known location
- Desktop environment provides autostart support (GNOME/KDE/XFCE/etc.)
- No existing `~/.config/autostart/kanata-switcher.desktop`

## Steps
1. Run `kanata-switcher --quiet-focus --install-autostart`
2. Verify `~/.config/autostart/kanata-switcher.desktop` exists
3. Inspect the file for absolute Exec path and expected args
4. Log out and log back in
5. Confirm daemon starts automatically and behaves as expected
6. Run `kanata-switcher --uninstall-autostart`
7. Verify the `.desktop` file is removed
8. Log out and log back in
9. Confirm daemon does not auto-start

## Expected Results
- Autostart file is created with absolute Exec path and passed daemon options
- Daemon launches on login when autostart file is present
- Autostart entry is removed cleanly
- Daemon no longer starts automatically after removal

Last tested: NOT TESTED
