{
  description = "Kanata-switcher Home Manager module build checks";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    kanata-switcher.url = "path:..";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      home-manager,
      kanata-switcher,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        checks = {
          home-module-build =
            (home-manager.lib.homeManagerConfiguration {
              inherit pkgs;
              modules = [
                kanata-switcher.homeModules.default
                {
                  services.kanata-switcher.enable = true;
                  home.username = "kanata-switcher-ci";
                  home.homeDirectory = "/home/kanata-switcher-ci";
                  home.stateVersion = "23.11";
                  manual = {
                    html.enable = false;
                    manpages.enable = false;
                    json.enable = false;
                  };
                }
              ];
            }).activationPackage;

          home-module-build-keyboards =
            (home-manager.lib.homeManagerConfiguration {
              inherit pkgs;
              modules = [
                kanata-switcher.homeModules.default
                {
                  services.kanata-switcher = {
                    enable = true;
                    keyboards = {
                      kinesis = {
                        kanataPort = 22334;
                        settings = [
                          { default = "default"; }
                          {
                            class = "code|codium|jetbrains";
                            layer = "terminal";
                          }
                        ];
                      };
                      framework13 = {
                        kanataPort = 22335;
                        logging = "none";
                        settings = [
                          { default = "default"; }
                          {
                            class = "kitty|alacritty|wezterm";
                            layer = "terminal";
                          }
                        ];
                      };
                    };
                  };
                  home.username = "kanata-switcher-ci";
                  home.homeDirectory = "/home/kanata-switcher-ci";
                  home.stateVersion = "23.11";
                  manual = {
                    html.enable = false;
                    manpages.enable = false;
                    json.enable = false;
                  };
                }
              ];
            }).activationPackage;

          nixos-module-build =
            (nixpkgs.lib.nixosSystem {
              inherit system;
              modules = [
                kanata-switcher.nixosModules.default
                {
                  services.kanata-switcher.enable = true;
                  fileSystems."/".device = "/dev/disk/by-label/ci-root";
                  fileSystems."/".fsType = "ext4";
                  boot.loader.grub.devices = [ "/dev/sda" ];
                  system.stateVersion = "23.11";
                }
              ];
            }).config.system.build.toplevel;

          nixos-module-build-keyboards =
            (nixpkgs.lib.nixosSystem {
              inherit system;
              modules = [
                kanata-switcher.nixosModules.default
                {
                  services.kanata-switcher = {
                    enable = true;
                    keyboards = {
                      kinesis = {
                        kanataPort = 22334;
                        settings = [
                          { default = "default"; }
                          {
                            class = "code|codium|jetbrains";
                            layer = "terminal";
                          }
                        ];
                      };
                      framework13 = {
                        kanataPort = 22335;
                        logging = "none";
                        settings = [
                          { default = "default"; }
                          {
                            class = "kitty|alacritty|wezterm";
                            layer = "terminal";
                          }
                        ];
                      };
                    };
                  };
                  fileSystems."/".device = "/dev/disk/by-label/ci-root";
                  fileSystems."/".fsType = "ext4";
                  boot.loader.grub.devices = [ "/dev/sda" ];
                  system.stateVersion = "23.11";
                }
              ];
            }).config.system.build.toplevel;

          nixos-module-keyboards-invalid-mixed =
            let
              evalResult = builtins.tryEval (
                (nixpkgs.lib.nixosSystem {
                  inherit system;
                  modules = [
                    kanata-switcher.nixosModules.default
                    {
                      services.kanata-switcher = {
                        enable = true;
                        kanataPort = 22334;
                        keyboards.kinesis = {
                          kanataPort = 22334;
                          settings = [ { default = "default"; } ];
                        };
                      };
                      fileSystems."/".device = "/dev/disk/by-label/ci-root";
                      fileSystems."/".fsType = "ext4";
                      boot.loader.grub.devices = [ "/dev/sda" ];
                      system.stateVersion = "23.11";
                    }
                  ];
                }).config.system.build.toplevel
              );
            in
            assert (!evalResult.success);
            pkgs.runCommand "nixos-module-keyboards-invalid-mixed" { } ''
              touch "$out"
            '';

          nixos-module-keyboards-invalid-config-and-settings =
            let
              evalResult = builtins.tryEval (
                (nixpkgs.lib.nixosSystem {
                  inherit system;
                  modules = [
                    kanata-switcher.nixosModules.default
                    {
                      services.kanata-switcher = {
                        enable = true;
                        keyboards.kinesis = {
                          kanataPort = 22334;
                          configFile = "${pkgs.writeText "kanata-switcher-test-config.json" "[]"}";
                          settings = [ { default = "default"; } ];
                        };
                      };
                      fileSystems."/".device = "/dev/disk/by-label/ci-root";
                      fileSystems."/".fsType = "ext4";
                      boot.loader.grub.devices = [ "/dev/sda" ];
                      system.stateVersion = "23.11";
                    }
                  ];
                }).config.system.build.toplevel
              );
            in
            assert (!evalResult.success);
            pkgs.runCommand "nixos-module-keyboards-invalid-config-and-settings" { } ''
              touch "$out"
            '';
        };
      }
    );
}
