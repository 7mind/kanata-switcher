# kanata-switcher

`kanata-switcher` provides support for switching [Kanata](https://github.com/jtroo/kanata) layers and pressing virtual keys based on the currently
focused application window for all Linux desktop environments - for Wayland: GNOME Shell, KDE Plasma, COSMIC, wlroots-based
compositors (Sway, Hyprland, Niri, etc.), and for X11.

As of the time when the project was started, the only active project for application-based layer switching for kanata
for Linux was [hyprkan](https://github.com/haithium/hyperkan) - which supported only wlroots-based compositors. There
was no project attempting support for GNOME Shell or KDE Plasma.

This project is fully LLM-generated, it has a comprehensive automated test suite and has also been manually tested in
the following environments:

- [x] GNOME Shell
- [x] KDE Plasma
- [x] COSMIC
- [x] wlroots-based compositors
    - [x] Sway
    - [x] Hyprland
    - [x] Niri
- [x] X11

There's also a manual QA checklist in [./qa](./qa) folder.

If you have tested it in other environments, and it did/didn't work, open a PR to change the README or QA checklist!

---

![ENTERPRISE QUALITY](./enterprise-quality.svg)

This project features comprehensive automated test suite and supports an unusually wide range of desktop environments in a single codebase.

---

## Machine summary

### Supported Environments

All environments use the unified daemon (`src/daemon/`). Backends are event-driven, with one-shot focus queries on startup and unpause.

| Environment                          | How it works                                                      |
|--------------------------------------|-------------------------------------------------------------------|
| GNOME Shell                          | Extension pushes focus changes to daemon via DBus                 |
| KDE Plasma                           | Daemon auto-injects KWin script which pushes via DBus             |
| COSMIC                               | Daemon receives `cosmic-toplevel-info` Wayland protocol events    |
| wlroots (Sway, Hyprland, Niri, etc.) | Daemon receives `wlr-foreign-toplevel-management` protocol events |
| X11                                  | Daemon listens to `PropertyNotify` events on `_NET_ACTIVE_WINDOW` |

### Prerequisites

1. Kanata running with TCP server enabled:
   ```bash
   kanata -c your-config.kbd -p 10000
   ```

2. Config file at `~/.config/kanata/kanata-switcher.json` (or in applicable `$XDG_CONFIG_HOME` location)

If systemd-logind is unavailable (no system bus, permissions, etc.), the daemon keeps running but native terminal switching is disabled; a warning is logged on startup.

For non-GNOME desktops, the SNI tray menu's "Show app layer only" setting is persisted via the GNOME extension's GSettings schema when available. Use `--indicator-focus-only true|false` to override it at startup.

### Config Format

Example config:

```json
[
  {
    "default": "default"
  },
  {
    "on_native_terminal": "tty"
  },
  {
    "class": "^firefox$",
    "layer": "browser"
  },
  {
    "class": "jetbrains|codium|code|dev.zed.Zed",
    "layer": "vscode"
  },
  {
    "class": "kitty|alacritty|com.mitchellh.ghostty|wezterm",
    "title": "vim",
    "layer": "vim"
  }
]
```

**Rule entries:**

- `class` - Window class regex (optional)
- `title` - Window title regex (optional)
- `layer` - Kanata layer name to switch to (optional)
- `virtual_key` - Virtual key to press while window is focused (optional, see below)
- `raw_vk_action` - Advanced: raw virtual key actions (optional, see below)
- `fallthrough` - Advanced: continue matching subsequent rules (optional, default false)
- Rules are evaluated top-to-bottom; a matching rule stops evaluation (unless it has `"fallthrough": true` attribute)
    - A matching rule with `"fallthrough": true` continues to subsequent rules; non-matching rules are skipped
    - All matching rules' actions are collected and execute in order (without any `"fallthrough": true` rules, that is exactly 0 or 1 action)
- Patterns use [Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax) (Perl-like, no lookahead/lookbehind)
- Use `*` as a special case to match anything

**Default layer rule:**

- `{ "default": "layer_name" }` - Explicit default layer (optional)
- When present, disables auto-detection from Kanata
- When absent, daemon auto-detects from Kanata's initial layer on connect
- Can appear at most once (multiple = error), position doesn't matter

**Native terminal rule:**

- `{ "on_native_terminal": "layer_name" }` - Layer to use when switching to Linux Kernel native terminal (Ctrl+Alt+F*)
- Must not include `class`, `title`, or `layer`
- Can include `virtual_key` and/or `raw_vk_action`
- Can appear at most once (multiple = error), position doesn't matter
- When absent, daemon switches to layer specified in Default layer rule or the auto-detected initial layer

**Virtual keys:**

- `virtual_key` - Automatically pressed when window is focused, released when unfocused
- With `"fallthrough": true`, ALL matching `virtual_key`s are pressed and held simultaneously
- VKs are pressed in rule order (top-to-bottom), released in reverse order (bottom-to-top)
- Example:
  ```json
  [
    {
      "class": "firefox",
      "virtual_key": "vk_browser",
      "layer": "browser"
    },
    {
      "class": "terminal",
      "virtual_key": "vk_terminal"
    }
  ]
  ```

**Raw virtual key actions:**

- `raw_vk_action` - Array of `[key_name, action]` pairs, fired on focus only (fire-and-forget)
- Actions:
  - `Press` - Press the key; remains pressed until another action triggers Release or Tap
  - `Release` - Release the key; does nothing if not pressed
  - `Tap` - Press and release the key; if already pressed, only releases it
  - `Toggle` - Press if not pressed, release if pressed
- Example:
  ```json
  [ 
    {
      "class": "firefox",
      "raw_vk_action": [["vk_notify", "Tap"], ["vk_browser", "Press"]],
      "fallthrough": true
    }
  ]
  ```

**Layer switching and stacking:**

- `"fallthrough": true` is only useful for virtual keys, not layers, because **only the last layer wins**, layer switches won't stack because kanata's TCP `ChangeLayer` command swaps the base layer (it doesn't stack)
- For **stacked layers** (e.g., browser layer + youtube-specific layer on top), use `virtual_key` with `layer-while-held` actions in your kanata config:

  In kanata config - define virtual keys with layer-while-held:
  ```lisp
  (defvirtualkeys
    vk_browser (layer-while-held browser)
    vk_youtube (layer-while-held youtube)
  )
  ```

  In kanata-switcher config - stack layers via virtual keys:
  ```json
  [
    {
      "class": "firefox",
      "virtual_key": "vk_browser",
      "fallthrough": true
    },
    {
      "class": "firefox",
      "title": "YouTube",
      "virtual_key": "vk_youtube"
    }
  ]
  ```

  When focusing Firefox on YouTube, both `vk_browser` and `vk_youtube` are held → kanata stacks `browser` and `youtube` layers.


### Running Without Installing

#### Nix

```bash
nix run github:7mind/kanata-switcher -- -p 10000
```

#### Cargo

```bash
cargo run --release -- -p 10000
```

**GNOME Shell note:** The daemon automatically installs and enables the required GNOME extension on first run. After
installation, restart GNOME Shell:

- **X11**: Press Alt+F2, type `r`, press Enter
- **Wayland**: Log out and log back in

The extension is loaded from the filesystem (`<install-dir>/gnome/`) if available, otherwise falls back to the embedded
copy (enabled by default via `embed-gnome-extension` cargo feature).

### Installing

#### Home Manager (Nix)

Add flake input and import module:

```nix
# flake.nix
{
  inputs.kanata-switcher.url = "github:7mind/kanata-switcher";

  outputs = { nixpkgs, home-manager, kanata-switcher, ... }: {
    homeConfigurations."user" = home-manager.lib.homeManagerConfiguration {
      modules = [
        kanata-switcher.homeModules.default
        # ...
      ];
    };
  };
}
```

Enable in your Home Manager config:

```nix
# home.nix
{ osConfig, ... }:  # if using home-manager as NixOS module
{
  services.kanata-switcher = {
    enable = true;
    kanataPort = 10000;  # optional, default 10000
    # kanataPort = osConfig.services.kanata.keyboards.default.port;  # if using programs.kanata from nixpkgs

    # Config - choose one:
    settings = [  # inline config (recommended)
      { default = "default"; }
      { class = "^firefox$"; layer = "browser"; }
      { class = "jetbrains|codium|code"; layer = "code"; }
    ];
    # configFile = ./kanata-switcher.json;  # Nix path, or string like "~/.config/..."
    # (neither) defaults to ~/.config/kanata/kanata-switcher.json

    # For GNOME Shell - choose one:
    gnomeExtension.enable = true;       # Nix-managed extension (recommended)
    # gnomeExtension.autoInstall = true; # Runtime auto-install (mutable)
    # gnomeExtension.manageDconf = false; # Disable dconf management (see below)

    # Logging for systemd unit:
    # logging = "quiet-focus"; # default: quiet-focus, options: quiet, quiet-focus, none
  };
}
```

#### NixOS Module (NixOS)

For system-wide installation without Home Manager:

```nix
# flake.nix
{
  inputs.kanata-switcher.url = "github:7mind/kanata-switcher";

  outputs = { nixpkgs, kanata-switcher, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      modules = [
        kanata-switcher.nixosModules.default
        # ...
      ];
    };
  };
}
```

```nix
# configuration.nix
{ config, ... }:
{
  services.kanata-switcher = {
    enable = true;
    kanataPort = 10000;  # optional, default 10000
    # kanataPort = config.services.kanata.keyboards.default.port;  # if using programs.kanata from nixpkgs

    # Config - choose one:
    settings = [  # inline config (recommended)
      { default = "default"; }
      { class = "^firefox$"; layer = "browser"; }
      { class = "jetbrains|codium|code"; layer = "code"; }
    ];
    # configFile = ./kanata-switcher.json;  # Nix path, or string like "~/.config/..."
    # (neither) defaults to ~/.config/kanata/kanata-switcher.json

    # For GNOME Shell:
    gnomeExtension.enable = true;  # installs extension and enables via dconf for all users
    # gnomeExtension.manageDconf = false; # Disable dconf management (see below)

    # Logging for systemd unit:
    # logging = "quiet-focus"; # default: quiet-focus, options: quiet, quiet-focus, none
  };
}
```

The NixOS module creates a systemd user service (`systemd.user.services`) that auto-starts for all users on graphical
login. Config file still defaults to per-user `~/.config/kanata/kanata-switcher.json`.

##### External GNOME Extension Management

When using a centralized GNOME extensions module that manages all extensions via locked dconf settings, this module's
dconf configuration will conflict - dconf databases don't merge, and locked settings take precedence.

Set `gnomeExtension.manageDconf = false` to disable dconf management:

```nix
# configuration.nix
{
  services.kanata-switcher = {
    enable = true;
    gnomeExtension.enable = true;
    gnomeExtension.manageDconf = false;  # Don't add dconf database entry
  };
}
```

Then include the extension UUID in your dconf enabled-extensions list. The extension package is already installed when
`gnomeExtension.enable = true`:

```nix
# your gnome-extensions module
{ config, ... }:
let
  kanataExtension = config.services.kanata-switcher.gnomeExtension.package;
in {
  programs.dconf.profiles.user.databases = [{
    lockAll = true;
    settings."org/gnome/shell".enabled-extensions = [
      # ... your other extensions ...
      kanataExtension.extensionUuid
    ];
  }];
}
```

#### Manual Installation (non-Nix)

For non-Nix / NixOS systems, install the binary and configure the systemd user service manually as follows.

1. Install the binary:
   ```bash
   cargo install --path .
   # Binary installed to ~/.cargo/bin/kanata-switcher
   ```

2. Copy the systemd unit file [`kanata-switcher.service`](./systemd/kanata-switcher.service):
   ```bash
   mkdir -p ~/.config/systemd/user
   cp systemd/kanata-switcher.service ~/.config/systemd/user/
   ```

3. The unit file works out of the box if `cargo install` installs to `~/.cargo/bin` (default) and with default kanata port 10000. Edit the systemd unit file if you use a different port or install binary to a different location.

4. Enable and start the service:
   ```bash
   systemctl --user daemon-reload
   systemctl --user enable --now kanata-switcher
   ```

#### Autostart (non-systemd)

If you don't use systemd user services or prefer GUI-managed autostart entries, install a user-level autostart entry:

```bash
~/.cargo/bin/kanata-switcher --quiet-focus -p 10000 --install-autostart
```

This writes `~/.config/autostart/kanata-switcher.desktop` with an absolute `Exec` path and the same daemon options you
passed on the command line. To update the entry, rerun the install command with new options. To remove it:

```bash
~/.cargo/bin/kanata-switcher --uninstall-autostart
```

### Daemon Options

```
-p, --port PORT                    Kanata TCP port (default: 10000)
-H, --host HOST                    Kanata host (default: 127.0.0.1)
-c, --config PATH                  Config file path
-q, --quiet                        Suppress focus/layer-switch messages
--quiet-focus                      Suppress focus messages only
--install-autostart                Install autostart desktop entry and exit
--uninstall-autostart              Uninstall autostart desktop entry and exit
--install-gnome-extension          Auto-install GNOME extension if missing (default)
--no-install-gnome-extension       Do not auto-install GNOME extension
--no-indicator                     Disable the StatusNotifier (SNI) indicator on non-GNOME desktops
--indicator-focus-only true|false  Override StatusNotifier (SNI) indicator focus-only mode
--restart                          Send Restart request to an existing daemon and exit
--pause                            Send Pause request to an existing daemon and exit
--unpause                          Send Unpause request to an existing daemon and exit
-h, --help                         Show help
```

Systemd units use `--quiet-focus` by default to reduce log noise.

### Related Projects

- [hyprkan](https://github.com/mdSlash/hyprkan) - Similar tool for Hyprland/Sway/Niri/X11
- [xremap](https://github.com/xremap/xremap) - Key remapper with per-app config
- [keymapper](https://github.com/houmain/keymapper) - Another key remapper with similar support
