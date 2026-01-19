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

  outputs = { self, nixpkgs, flake-utils, home-manager, kanata-switcher }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
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
        };
      });
}
