# kanata-switcher

`kanata-switcher` provides support for switching [Kanata](https://github.com/jtroo/kanata) layers based on currently focused application windows for Linux Wayland desktop environments - GNOME Shell, KDE Plasma, COSMIC and wlroots-based compositors (Sway, Hyprland, Niri, etc.).

As of the time when the project was started, the only active project for application-based layer switching for kanata for Linux was [hyprkan](https://github.com/haithium/hyperkan) - which supported only wlroots-based compositors. There was no project attempting support for GNOME Shell or KDE Plasma.

This project aims to support all Wayland-based environments at once in a single application. And possibly X11 too, but later!

This project is fully LLM-generated and has so far been tested manually on the following environments:

- [x] GNOME Shell
- [x] KDE Plasma
- [x] COSMIC
- [ ] wlroots-based compositors
    - [ ] Sway
    - [ ] Hyprland
    - [ ] Niri

If you have tested it in other environments, and it did/didn't work, open a PR to change the README!

## Machine summary

### Supported Environments

All environments use the unified daemon (`src/daemon/`).

| Environment | How it works |
|-------------|--------------|
| GNOME Shell | Daemon polls GNOME extension via DBus |
| KDE Plasma | Daemon auto-injects KWin script at runtime |
| COSMIC | Daemon uses `cosmic-toplevel-info` Wayland protocol |
| wlroots (Sway, Hyprland, Niri, etc.) | Daemon uses `wlr-foreign-toplevel-management` Wayland protocol |

### Prerequisites

1. Kanata running with TCP server enabled:
   ```bash
   kanata -c your-config.kbd -p 10000
   ```

2. Config file at usually `~/.config/kanata/kanata-switcher.json` (or in applicable `$XDG_CONFIG_HOME`)

### Config Format

Example config:

```json
[
  { "default": "default" },
  { "class": "^firefox$", "layer": "browser" },
  { "class": "jetbrains|codium|code|dev.zed.Zed", "layer": "vscode" },
  { "class": "kitty|alacritty|com.mitchellh.ghostty|wezterm", "title": "vim", "layer": "vim" }
]
```

**Rule entries:**
- `class` - Window class regex (optional)
- `title` - Window title regex (optional)
- `layer` - Kanata layer name to switch to
- Rules are matched top-to-bottom, first match wins
- Patterns use [Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax) (Perl-like, no lookahead/lookbehind)
- Use `*` as a special case to match anything

**Default layer:**
- `{ "default": "layer_name" }` - Explicit default layer (optional)
- When present, disables auto-detection from Kanata
- When absent, daemon auto-detects from Kanata's initial layer on connect
- Can appear at most once (multiple = error), position doesn't matter

### Running Without Installing

#### Nix

```bash
nix run github:7mind/kanata-switcher -- -p 10000
```

#### Cargo

```bash
cargo run --release -- -p 10000
```

**GNOME Shell note:** The daemon automatically installs and enables the required GNOME extension on first run. After installation, restart GNOME Shell:
- **X11**: Press Alt+F2, type `r`, press Enter
- **Wayland**: Log out and log back in

The extension is loaded from the filesystem (`<install-dir>/gnome/`) if available, otherwise falls back to the embedded copy (enabled by default via `embed-gnome-extension` cargo feature).

### Installing

#### Home Manager (NixOS / Nix)

Add flake input and import module:

```nix
# flake.nix
{
  inputs.kanata-switcher.url = "github:7mind/kanata-switcher";

  outputs = { nixpkgs, home-manager, kanata-switcher, ... }: {
    homeConfigurations."user" = home-manager.lib.homeManagerConfiguration {
      modules = [
        kanata-switcher.homeManagerModules.default
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
  };
}
```

#### NixOS Module

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
  };
}
```

The NixOS module creates a systemd user service (`systemd.user.services`) that auto-starts for all users on graphical login. Config file still defaults to per-user `~/.config/kanata/kanata-switcher.json`.

#### External GNOME Extension Management

When using a centralized GNOME extensions module that manages all extensions via locked dconf settings, this module's dconf configuration will conflict - dconf databases don't merge, and locked settings take precedence.

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

Then include the extension UUID in your dconf enabled-extensions list. The extension package is already installed when `gnomeExtension.enable = true`:

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

For non-Nix systems, install the binary and configure the systemd user service manually.

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

3. The unit file works out of the box if `cargo install` installs to `~/.cargo/bin` (default) and with kanata port 10000. Edit it if using non-default paths or port.

4. Enable and start the service:
   ```bash
   systemctl --user daemon-reload
   systemctl --user enable --now kanata-switcher
   ```

### Daemon Options

```
-p, --port PORT              Kanata TCP port (default: 10000)
-H, --host HOST              Kanata host (default: 127.0.0.1)
-c, --config PATH            Config file path
-q, --quiet                  Suppress focus/layer-switch messages
--install-gnome-extension    Auto-install GNOME extension if missing (default)
--no-install-gnome-extension Do not auto-install GNOME extension
-h, --help                   Show help
```

Systemd units use `--quiet` by default to reduce log noise.

### How It Works

Single daemon handles all environments:

- **GNOME**: Daemon polls extension via DBus → extension returns focused window
- **KDE**: Daemon injects KWin script → script calls daemon via DBus on focus change
- **COSMIC**: Daemon connects to Wayland and uses `cosmic-toplevel-info` protocol (separate from wlroots, as COSMIC is not wlroots-based)
- **wlroots compositors**: Daemon connects to Wayland and uses `wlr-foreign-toplevel-management` protocol to receive focus events

### Related Projects

- [hyprkan](https://github.com/mdSlash/hyprkan) - Similar tool for Hyprland/Sway/Niri/X11
- [xremap](https://github.com/xremap/xremap) - Key remapper with per-app config
- [keymapper](https://github.com/houmain/keymapper) - Another key remapper with similar support
