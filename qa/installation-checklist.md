# Installation Checklist

Last tested: 2026-01-22
Environment: NixOS, KDE, Wayland

## Cargo install
- [ ] `cargo install --path .` succeeds
- [ ] Binary is on PATH (or invoked directly)
- [x] `kanata-switcher --help` shows CLI options

## Nix install
- [x] `nix build` succeeds
- [x] `nix run` starts the daemon

## GNOME extension auto-install (GNOME only)
- [ ] First run installs extension without manual steps
- [ ] GNOME Shell restart instructions are shown
- [ ] After restart, extension is enabled

## Manual GNOME extension install (if auto-install disabled)
- [ ] `--no-install-gnome-extension` skips auto-install
- [ ] Manual `gnome-extensions pack/install/enable` works

## Config discovery
- [x] Default config path `~/.config/kanata/kanata-switcher.json` is used
- [ ] Missing config errors show example config
- [ ] Explicit `--config` path is honored
