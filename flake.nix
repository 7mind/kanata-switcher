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

            nativeBuildInputs = [ pkgs.buildPackages.glib.dev ];

            installPhase = ''
              runHook preInstall

              extensionDir=$out/share/gnome-shell/extensions/kanata-switcher@7mind.io
              mkdir -p $extensionDir/schemas
              cp *.js metadata.json $extensionDir/
              cp schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml $extensionDir/schemas/
              ${pkgs.buildPackages.glib.dev}/bin/glib-compile-schemas $extensionDir/schemas

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
            nativeBuildInputs = [ pkgs.pkg-config pkgs.buildPackages.glib.dev ];
          };

          rustDaemonCargoArtifacts = craneLib.buildDepsOnly rustDaemonCommonArgs;

          kanata-switcher-daemon = craneLib.buildPackage (rustDaemonCommonArgs // {
            cargoArtifacts = rustDaemonCargoArtifacts;

            doCheck = false;

            # Disable embedded extension since we bundle files alongside binary
            cargoExtraArgs = "--no-default-features";

            postInstall = ''
              mkdir -p $out/bin/gnome
              mkdir -p $out/bin/gnome/schemas
              cp ${./src/gnome-extension}/*.js $out/bin/gnome/
              cp ${./src/gnome-extension}/metadata.json $out/bin/gnome/
              cp ${./src/gnome-extension}/schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml $out/bin/gnome/schemas/
              ${pkgs.buildPackages.glib.dev}/bin/glib-compile-schemas $out/bin/gnome/schemas
            '';

            meta = with pkgs.lib; {
              description = "Daemon for switching kanata layers based on focused window";
              license = licenses.mit;
              mainProgram = "kanata-switcher";
            };
          });

          # Test archive - compile tests into nextest archive (cached)
          kanata-switcher-test-archive = craneLib.mkCargoDerivation (rustDaemonCommonArgs // {
            pname = "kanata-switcher-test-archive";
            cargoArtifacts = rustDaemonCargoArtifacts;
            nativeBuildInputs = rustDaemonCommonArgs.nativeBuildInputs ++ [ pkgs.cargo-nextest ];

            # Build test archive without running
            buildPhaseCargoCommand = ''
              mkdir -p $out
              cargo nextest archive --release --archive-file $out/archive.tar.zst
            '';

            installPhaseCommand = "true";  # Archive created in build phase
            doInstallCargoArtifacts = false;
          });

          # Script to run tests from nextest archive
          # Runs from temp directory with minimal Cargo.toml so nextest can write output files
          run-tests = pkgs.writeShellScriptBin "run-tests" ''
            WORK_DIR=$(mktemp -d)
            trap 'rm -rf "$WORK_DIR"' EXIT
            cd "$WORK_DIR"
            echo '[workspace]' > Cargo.toml
            HOME="$WORK_DIR" ${pkgs.xvfb-run}/bin/xvfb-run -s "-screen 0 800x600x24" \
              ${pkgs.cargo-nextest}/bin/cargo-nextest nextest run \
              --no-fail-fast \
              --archive-file ${kanata-switcher-test-archive}/archive.tar.zst \
              --workspace-remap . "$@"
          '';

          # Check derivation that runs tests (reuses run-tests script)
          # Adds dbus-daemon to PATH for DBus integration tests
          kanata-switcher-tests = pkgs.runCommand "kanata-switcher-tests" {
            nativeBuildInputs = [ pkgs.dbus ];
          } ''
            ${run-tests}/bin/run-tests
            touch $out
          '';

        in {
          packages = {
            daemon = kanata-switcher-daemon;
            gnome-extension = kanata-switcher-gnome-extension;
            default = kanata-switcher-daemon;
          };

          checks = {
            tests = kanata-switcher-tests;
            nixos-module-build =
              (nixpkgs.lib.nixosSystem {
                inherit system;
                modules = [
                  self.nixosModules.default
                  {
                    services.kanata-switcher.enable = true;
                    boot.isContainer = true;
                    fileSystems."/" = {
                      device = "tmpfs";
                      fsType = "tmpfs";
                    };
                    system.stateVersion = "23.11";
                  }
                ];
              }).config.system.build.toplevel;
            gnome-schema = pkgs.runCommand "kanata-switcher-gnome-schema-check" {} ''
              test -f ${kanata-switcher-gnome-extension}/share/gnome-shell/extensions/kanata-switcher@7mind.io/schemas/gschemas.compiled
              touch $out
            '';
            gnome-extension-files = pkgs.runCommand "kanata-switcher-gnome-extension-files-check" {} ''
              set -euo pipefail

              expected_js=$(
                cd ${./src/gnome-extension}
                find . -maxdepth 1 -type f -name "*.js" -printf "%f\n" | sort
              )

              extension_dir=${kanata-switcher-gnome-extension}/share/gnome-shell/extensions/kanata-switcher@7mind.io
              actual_js_extension=$(
                cd "$extension_dir"
                find . -maxdepth 1 -type f -name "*.js" -printf "%f\n" | sort
              )
              if [ "$expected_js" != "$actual_js_extension" ]; then
                echo "GNOME extension JS mismatch in gnome-extension package"
                echo "expected:"
                echo "$expected_js"
                echo "actual:"
                echo "$actual_js_extension"
                exit 1
              fi

              daemon_gnome_dir=${kanata-switcher-daemon}/bin/gnome
              actual_js_daemon=$(
                cd "$daemon_gnome_dir"
                find . -maxdepth 1 -type f -name "*.js" -printf "%f\n" | sort
              )
              if [ "$expected_js" != "$actual_js_daemon" ]; then
                echo "GNOME extension JS mismatch in daemon package"
                echo "expected:"
                echo "$expected_js"
                echo "actual:"
                echo "$actual_js_daemon"
                exit 1
              fi

              test -f "$extension_dir/metadata.json"
              test -f "$daemon_gnome_dir/metadata.json"
              touch $out
            '';
            gnome-format = pkgs.runCommand "kanata-switcher-gnome-format-check" {
              nativeBuildInputs = [ pkgs.gjs ];
            } ''
              KANATA_SWITCHER_SRC=${./.} ${pkgs.gjs}/bin/gjs -m ${./tests/gnome-extension-format.js}
              KANATA_SWITCHER_SRC=${./.} ${pkgs.gjs}/bin/gjs -m ${./tests/gnome-extension-dbus.js}
              KANATA_SWITCHER_SRC=${./.} ${pkgs.gjs}/bin/gjs -m ${./tests/gnome-extension-daemon-state.js}
              KANATA_SWITCHER_SRC=${./.} ${pkgs.gjs}/bin/gjs -m ${./tests/gnome-extension-focus.js}
              touch $out
            '';
          };

          apps.test = {
            type = "app";
            program = "${run-tests}/bin/run-tests";
          };

          devShells.default = pkgs.mkShell {
            buildInputs = with pkgs; [
              gnome-extensions-cli
              glib
              dbus
              rustToolchain
              rust-analyzer
              pkg-config
              # For X11 integration tests
              xorg.xorgserver  # provides Xvfb
              xvfb-run
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

    in perSystem // (let
      moduleOptions = lib: packages: {
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
          type = lib.types.nullOr (lib.types.either lib.types.path lib.types.str);
          default = null;
          example = "~/.config/kanata/kanata-switcher.json";
          description = "Path to config file. Mutually exclusive with 'settings'. Defaults to ~/.config/kanata/kanata-switcher.json when neither is set.";
        };

        settings = lib.mkOption {
          type = lib.types.nullOr (lib.types.listOf lib.types.attrs);
          default = null;
          example = lib.literalExpression ''
            [
              { default = "default"; }
              { class = "^firefox$"; layer = "browser"; }
              { class = "jetbrains|codium|code"; layer = "code"; }
              { class = "kitty|alacritty"; layer = "terminal"; }
            ]
          '';
          description = "Config as a list of rule attrsets, serialized to JSON. Mutually exclusive with 'configFile'.";
        };

        logging = lib.mkOption {
          type = lib.types.enum [ "quiet" "quiet-focus" "none" ];
          default = "quiet-focus";
          description = "Log verbosity for systemd units. quiet = suppress focus + layer logs, quiet-focus = suppress focus logs only, none = no suppression.";
        };

        gnomeExtension = {
          enable = lib.mkEnableOption "GNOME Shell extension for kanata-switcher (Nix-managed)";

          autoInstall = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Auto-install GNOME extension at runtime (for mutable config). When false, use gnomeExtension.enable for Nix-managed installation.";
          };

          manageDconf = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Whether to manage dconf/GNOME Shell enabled-extensions. Set to false when using external extension management (e.g., a centralized gnome-extensions module with locked dconf settings).";
          };

          package = lib.mkOption {
            type = lib.types.package;
            default = packages.gnome-extension;
            description = "kanata-switcher GNOME extension package";
          };
        };
      };

      mkModule = mkConfig: { config, lib, pkgs, ... }:
        let
          cfg = config.services.kanata-switcher;
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
          configFile =
            if cfg.configFile != null then cfg.configFile
            else if cfg.settings != null then pkgs.writeText "kanata-switcher.json" (builtins.toJSON cfg.settings)
            else null;
          loggingArg =
            if cfg.logging == "quiet" then "--quiet"
            else if cfg.logging == "quiet-focus" then "--quiet-focus"
            else null;
          execArgs = [
            "${cfg.package}/bin/kanata-switcher"
            "-p" (toString cfg.kanataPort)
            "-H" cfg.kanataHost
          ] ++ lib.optionals (loggingArg != null) [ loggingArg ]
            ++ lib.optionals (configFile != null) [ "-c" (toString configFile) ]
            ++ lib.optionals (!cfg.gnomeExtension.autoInstall) [ "--no-install-gnome-extension" ];
        in {
          options.services.kanata-switcher = moduleOptions lib packages;
          config = lib.mkIf cfg.enable ({
            assertions = [{
              assertion = cfg.configFile == null || cfg.settings == null;
              message = "services.kanata-switcher: 'configFile' and 'settings' are mutually exclusive";
            }];
          } // mkConfig cfg lib execArgs);
        };

    in {
      lib.moduleOptions = moduleOptions;

      nixosModules.default = mkModule (cfg: lib: execArgs: {
        environment.systemPackages = [ cfg.package ]
          ++ lib.optionals cfg.gnomeExtension.enable [ cfg.gnomeExtension.package ];

        systemd.user.services.kanata-switcher = {
          description = "Kanata layer switcher daemon";
          after = [ "graphical-session.target" ];
          partOf = [ "graphical-session.target" ];
          wantedBy = [ "graphical-session.target" ];
          serviceConfig = {
            Type = "simple";
            ExecStart = lib.concatStringsSep " " execArgs;
            Restart = "on-failure";
            RestartSec = 5;
          };
          environment.XDG_DATA_DIRS = "/run/current-system/sw/share";
          # restartTriggers adds X-Restart-Triggers to unit file, but NixOS doesn't
          # process it for user services - only system services are handled.
          # See: https://github.com/NixOS/nixpkgs/issues/246611
          restartTriggers = [
            (builtins.toJSON { settings = cfg.settings; configFile = cfg.configFile; })
          ];
        };

        programs.dconf = lib.mkIf (cfg.gnomeExtension.enable && cfg.gnomeExtension.manageDconf) {
          enable = true;
          profiles.user.databases = [{
            settings."org/gnome/shell".enabled-extensions = [ "kanata-switcher@7mind.io" ];
          }];
        };
      });

      homeModules.default = mkModule (cfg: lib: execArgs: {
        home.packages = [ cfg.package ]
          ++ lib.optionals cfg.gnomeExtension.enable [ cfg.gnomeExtension.package ];

        systemd.user.services.kanata-switcher = {
          Unit = {
            Description = "Kanata layer switcher daemon";
            After = [ "graphical-session.target" ];
            PartOf = [ "graphical-session.target" ];
            X-Restart-Triggers = [
              (toString (builtins.toJSON { settings = cfg.settings; configFile = cfg.configFile; }))
            ];
          };
          Service = {
            Type = "simple";
            ExecStart = lib.concatStringsSep " " execArgs;
            Restart = "on-failure";
            RestartSec = 5;
            Environment = [ "XDG_DATA_DIRS=%h/.nix-profile/share:/run/current-system/sw/share" ];
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };

        dconf.settings = lib.mkIf (cfg.gnomeExtension.enable && cfg.gnomeExtension.manageDconf) {
          "org/gnome/shell".enabled-extensions = [ "kanata-switcher@7mind.io" ];
        };
      });
    });
}
