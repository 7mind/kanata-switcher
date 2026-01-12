{
  description = "Kanata layer switcher based on focused application";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay }:
    let
      # Packages per system
      perSystem = flake-utils.lib.eachDefaultSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          rustToolchain = pkgs.rust-bin.stable.latest.default;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

          kanata-switcher-gnome-extension = pkgs.stdenv.mkDerivation {
            pname = "gnome-shell-extension-kanata-switcher";
            version = "0.1.0";

            src = ./src/gnome-extension;

            nativeBuildInputs = [ pkgs.buildPackages.glib ];

            installPhase = ''
              runHook preInstall

              extensionDir=$out/share/gnome-shell/extensions/kanata-switcher@7mind.io
              mkdir -p $extensionDir
              cp extension.js metadata.json $extensionDir/

              runHook postInstall
            '';

            passthru.extensionUuid = "kanata-switcher@7mind.io";

            meta = with pkgs.lib; {
              description = "GNOME Shell extension for kanata-switcher";
              license = licenses.mit;
            };
          };

          # Rust daemon - include src/protocols/*.xml for wayland-scanner and src/gnome-extension/* for build.rs
          rustDaemonSrc = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              (craneLib.filterCargoSources path type) ||
              (builtins.match ".*/src/protocols/.*\\.xml$" path != null) ||
              (builtins.match ".*/src/gnome-extension/.*" path != null);
          };

          rustDaemonCommonArgs = {
            src = rustDaemonSrc;
            strictDeps = true;
            buildInputs = [ pkgs.dbus ];
            nativeBuildInputs = [ pkgs.pkg-config ];
          };

          rustDaemonCargoArtifacts = craneLib.buildDepsOnly rustDaemonCommonArgs;

          kanata-switcher-daemon = craneLib.buildPackage (rustDaemonCommonArgs // {
            cargoArtifacts = rustDaemonCargoArtifacts;

            doCheck = false;

            # Disable embedded extension since we bundle files alongside binary
            cargoExtraArgs = "--no-default-features";

            postInstall = ''
              mkdir -p $out/bin/gnome
              cp ${./src/gnome-extension}/extension.js $out/bin/gnome/
              cp ${./src/gnome-extension}/metadata.json $out/bin/gnome/
            '';

            meta = with pkgs.lib; {
              description = "Daemon for switching kanata layers based on focused window";
              license = licenses.mit;
              mainProgram = "kanata-switcher";
            };
          });

        in {
          packages = {
            daemon = kanata-switcher-daemon;
            gnome-extension = kanata-switcher-gnome-extension;
            default = kanata-switcher-daemon;
          };

          devShells.default = pkgs.mkShell {
            buildInputs = with pkgs; [
              gnome-extensions-cli
              glib
              dbus
              rustToolchain
              rust-analyzer
              pkg-config
            ];

            shellHook = ''
              echo "kanata-switcher development environment"
              echo "  cargo build                        Build daemon"
              echo "  cargo run -- -p 10000              Run daemon"
              echo "  nix build                          Build with Nix"
              echo "  ./install-gnome-shell-ext.sh       Install GNOME extension"
            '';
          };
        }
      );

    in perSystem // {
      # Shared module options (used by both NixOS and Home Manager modules)
      lib.moduleOptions = lib: packages: {
        enable = lib.mkEnableOption "kanata-switcher daemon";

        package = lib.mkOption {
          type = lib.types.package;
          default = packages.daemon;
          description = "kanata-switcher daemon package";
        };

        kanataPort = lib.mkOption {
          type = lib.types.port;
          default = 10000;
          description = "Kanata TCP port";
        };

        kanataHost = lib.mkOption {
          type = lib.types.str;
          default = "127.0.0.1";
          description = "Kanata host address";
        };

        configFile = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          description = "Path to config file. Defaults to ~/.config/kanata/kanata-switcher.json";
        };

        gnomeExtension = {
          enable = lib.mkEnableOption "GNOME Shell extension for kanata-switcher (Nix-managed)";

          autoInstall = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Auto-install GNOME extension at runtime (for mutable config). When false, use gnomeExtension.enable for Nix-managed installation.";
          };

          package = lib.mkOption {
            type = lib.types.package;
            default = packages.gnome-extension;
            description = "kanata-switcher GNOME extension package";
          };
        };
      };

      # Shared: build ExecStart args
      lib.mkExecArgs = cfg: [
        "${cfg.package}/bin/kanata-switcher"
        "-p" (toString cfg.kanataPort)
        "-H" cfg.kanataHost
      ] ++ (if cfg.configFile != null then [ "-c" (toString cfg.configFile) ] else [])
        ++ (if !cfg.gnomeExtension.autoInstall then [ "--no-install-gnome-extension" ] else []);

      # NixOS module
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.kanata-switcher;
          packages = self.packages.${pkgs.system};
        in {
          options.services.kanata-switcher = self.lib.moduleOptions lib packages;

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ]
              ++ lib.optionals cfg.gnomeExtension.enable [ cfg.gnomeExtension.package ];

            systemd.user.services.kanata-switcher = {
              description = "Kanata layer switcher daemon";
              after = [ "graphical-session.target" ];
              partOf = [ "graphical-session.target" ];
              wantedBy = [ "graphical-session.target" ];
              serviceConfig = {
                Type = "simple";
                ExecStart = lib.concatStringsSep " " (self.lib.mkExecArgs cfg);
                Restart = "on-failure";
                RestartSec = 5;
              };
            };

            programs.dconf = lib.mkIf cfg.gnomeExtension.enable {
              enable = true;
              profiles.user.databases = [{
                settings."org/gnome/shell".enabled-extensions = [ "kanata-switcher@7mind.io" ];
              }];
            };
          };
        };

      # Home Manager module
      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.kanata-switcher;
          packages = self.packages.${pkgs.system};
        in {
          options.services.kanata-switcher = self.lib.moduleOptions lib packages;

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ]
              ++ lib.optionals cfg.gnomeExtension.enable [ cfg.gnomeExtension.package ];

            systemd.user.services.kanata-switcher = {
              Unit = {
                Description = "Kanata layer switcher daemon";
                After = [ "graphical-session.target" ];
                PartOf = [ "graphical-session.target" ];
              };
              Service = {
                Type = "simple";
                ExecStart = lib.concatStringsSep " " (self.lib.mkExecArgs cfg);
                Restart = "on-failure";
                RestartSec = 5;
              };
              Install.WantedBy = [ "graphical-session.target" ];
            };

            dconf.settings = lib.mkIf cfg.gnomeExtension.enable {
              "org/gnome/shell" = {
                enabled-extensions = [ "kanata-switcher@7mind.io" ];
              };
            };
          };
        };
    };
}
