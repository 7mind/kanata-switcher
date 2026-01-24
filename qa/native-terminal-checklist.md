# Native Terminal Focus Checklist

Last tested: 2026-01-24
Environment: NixOS, GNOME, Wayland

## Native Terminal Switch
- [x] Switch from GUI to a native terminal (Ctrl+Alt+F3) while focused on a window with a layer rule; confirm layer switches to `on_native_terminal` and VKs update
- [x] Switch back to the GUI session; confirm focus layer refreshes to the active window without waiting for a new focus event
- [x] Verify behavior when no `on_native_terminal` rule exists (should switch to default layer on native terminal)
- [x] Confirm focus refresh when returning to GUI from a native terminal after being focused on a terminal window
- [x] Verify pause mode ignores native terminal transitions and resumes normal behavior when unpaused
