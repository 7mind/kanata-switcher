# QA Checklist - DBus Multi-Instance

Human testing for the per-instance DBus name + multi-indicator behavior.

## Setup

Configure two daemons via Home Manager / NixOS:

```nix
services.kanata-switcher = {
  enable = true;
  keyboards = {
    kinesis = {
      kanataPort = 22334;
      settings = [ { default = "default"; } ];
    };
    framework13 = {
      kanataPort = 22335;
      settings = [ { default = "default"; } ];
    };
  };
};
```

Start both kanata instances on the matching ports.

## Test scenarios

### Bus name registration

- [ ] `busctl --user list | grep com.github.kanata.Switcher` shows
      `com.github.kanata.Switcher.instances.kinesis` and
      `com.github.kanata.Switcher.instances.framework13`, and nothing
      else under `com.github.kanata.Switcher.*`.
- [ ] No owner for the legacy bare `com.github.kanata.Switcher` name.

### Single-instance default suffix

- [ ] Daemon started with no `--dbus-suffix` and default host/port owns
      `com.github.kanata.Switcher.instances.p10000`.
- [ ] Daemon started with `-p 22334` (no `--dbus-suffix`) owns
      `com.github.kanata.Switcher.instances.p22334`.

### GNOME multi-indicator UI

- [ ] On GNOME Shell, two top-bar indicators appear (one per keyboard).
- [ ] Panel labels show only the layer letter + VK glyph — no keyboard
      prefix visible.
- [ ] Hovering each indicator (or using a screen reader) reveals the
      keyboard name (`Keyboard: kinesis` / `Keyboard: framework13`).
- [ ] Per-indicator `Pause` toggle: only the targeted daemon's status
      switches to paused; the other indicator remains active.
- [ ] Per-indicator `Restart` action makes only the targeted indicator
      briefly disappear and reappear.

### Control CLI

- [ ] `kanata-switcher --dbus-suffix kinesis --pause` pauses only the
      `kinesis` daemon. CLI prints `[Control] Sent pause request to
      com.github.kanata.Switcher.instances.kinesis`.
- [ ] `kanata-switcher --pause` (no suffix) pauses **both** daemons and
      prints one line per daemon in `[Control] <name>: pause ok` form.
- [ ] `kanata-switcher --unpause` mirrors the broadcast behavior.
- [ ] `kanata-switcher --dbus-suffix doesnotexist --pause` exits with a
      clear error mentioning the resolved name.
- [ ] `kanata-switcher --pause` with no daemons running prints
      "No daemons running" and exits non-zero.

### KDE

- [ ] On KDE Plasma with two daemons running, focusing a window matching
      keyboard A's rules switches kanata A's layer; focusing a window
      matching keyboard B's rules switches kanata B's layer. Both
      daemons observe focus events independently.

### GNOME extension lifecycle

- [ ] Starting a third daemon (with another suffix) at runtime: a third
      top-bar indicator appears within a second without restarting GNOME
      Shell.
- [ ] Killing a daemon at runtime: its indicator disappears.

### Single-keyboard config

- [ ] No `keyboards` block, default flake settings: daemon registers
      `com.github.kanata.Switcher.instances.p10000`. One indicator
      appears with keyboard tooltip `p10000`.
