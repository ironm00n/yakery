{
  description = "ironmoon's NixOS configuration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05";
    nixpkgs-24_11.url = "github:nixos/nixpkgs/nixos-24.11"; # needed for twemoji-colr
    systems.url = "github:nix-systems/default";
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    plasma-manager = {
      url = "github:nix-community/plasma-manager";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
    };
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    binary-ninja = {
      url = "github:jchv/nix-binary-ninja";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  nixConfig = {
    extra-substituters = [
      "https://nixpkgs-unfree.cachix.org"
      "https://numtide.cachix.org"
      "https://ironmoon.cachix.org"
      "https://hyprland.cachix.org"
      "https://anmonteiro.nix-cache.workers.dev"
    ];
    extra-trusted-public-keys = [
      "nixpkgs-unfree.cachix.org-1:hqvoInulhbV4nJ9yJOEr+4wxhDV4xq2d1DK7S6Nj6rs="
      "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      "ironmoon.cachix.org-1:wowGL4TAzZPBO0fCqOekQLFqim3iXzdR+hIrK/tUadI="
      "hyprland.cachix.org-1:a7pgxzMz7+chwVL3/pzj6jIBMioiJM7ypFP8PwtkuGc="
      "ocaml.nix-cache.com-1:/xI2h2+56rwFfKyyFVbkJSeGqSIYMC/Je+7XXqGKDIY="
    ];
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      nixpkgs-stable,
      home-manager,
      plasma-manager,
      nixos-hardware,
      treefmt-nix,
      binary-ninja,
      systems,
      ...
    }:
    let
      overlays = import ./overlays/default.nix;
      lib = nixpkgs.lib;
      eachSystem =
        f: lib.genAttrs (import systems) (system: f (import nixpkgs { inherit system overlays; }));
      treefmtEval = eachSystem (pkgs: treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix);
    in
    let
      base-nixpkgs-config = {
        allowUnfree = true;
      };
      nixConfig = {
        nixpkgs.overlays = overlays;
        nixpkgs.config = base-nixpkgs-config;

        nix.settings.experimental-features = [
          "nix-command"
          "flakes"
        ];
      };
      base-modules = [
        nixConfig
        ./hosts/options.nix
        home-manager.nixosModules.home-manager
        binary-ninja.nixosModules.binaryninja
      ];
      system = "x86_64-linux";
      base-system = rec {
        inherit system;
        specialArgs = ({
          inherit inputs system;
          pkgs-stable = import nixpkgs-stable {
            inherit system;
            config = base-nixpkgs-config;
          };
        });
      };
    in
    {
      nixosConfigurations = {
        framework = lib.nixosSystem (
          base-system
          // {
            modules = base-modules ++ [
              nixos-hardware.nixosModules.framework-13-7040-amd
              ./hosts/framework/configuration.nix
            ];
          }
        );
        desktop = lib.nixosSystem (
          base-system
          // {
            modules = base-modules ++ [
              ./hosts/desktop/configuration.nix
            ];
          }
        );
      };

      homeConfigurations = {
        "ironmoon" = home-manager.lib.homeManagerConfiguration {
          # todo make this nicer
          pkgs = import nixpkgs { inherit system overlays; };
          extraSpecialArgs = { inherit inputs; };
          modules = [
            {
              home.username = "ironmoon";
              home.homeDirectory = "/home/ironmoon";
            }
            plasma-manager.homeManagerModules.plasma-manager
            ./users/ironmoon/home-manager.nix
          ];
        };
      };

      formatter = eachSystem (pkgs: treefmtEval.${pkgs.system}.config.build.wrapper);

      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs =
            [
              treefmtEval.${pkgs.system}.config.build.wrapper
            ]
            ++ (with pkgs; [
              nixd
              nixfmt-rfc-style
              nil

              lua-language-server
            ]);

          shellHook = ''
            export ROOT_NIXOS_PATH=$(git rev-parse --show-toplevel)
            echo $ROOT_NIXOS_PATH
          '';
        };
      });
    };
}
