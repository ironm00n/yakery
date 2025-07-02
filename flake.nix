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
      url = "github:ironm00n/nix-binary-ninja";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    quickshell = {
      url = "git+https://git.outfoxxed.me/outfoxxed/quickshell";
      inputs.nixpkgs.follows = "nixpkgs";
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
      flake-utils,
      ...
    }:
    let
      overlays = import ./overlays/default.nix;
      inherit (nixpkgs) lib;
      all-systems = import systems;
      base-nixpkgs-config = {
        allowUnfree = true;
      };
      pkgs-map =
        all-systems
        |> map (system: {
          name = system;
          value = import nixpkgs {
            inherit system overlays;
            config = base-nixpkgs-config;
          };
        })
        |> builtins.listToAttrs;
      eachSystem = f: lib.genAttrs all-systems (system: f pkgs-map.${system});
      treefmtEval = eachSystem (pkgs: treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix);
      mk-pkgs-stable =
        system:
        import nixpkgs-stable {
          inherit system;
          config = base-nixpkgs-config;
        };
    in
    let
      base-config =
        { pkgs, host }:
        {
          host = host;
          nixpkgs.pkgs = pkgs;

          nix.settings.experimental-features = [
            "nix-command"
            "flakes"
            "pipe-operators"
            "no-url-literals"
          ];
        };
      base-modules = ctx: [
        ./hosts/options.nix
        (base-config ctx)
        home-manager.nixosModules.home-manager
        binary-ninja.nixosModules.binaryninja
      ];
      base-system = system: {
        inherit system;
        specialArgs = {
          inherit inputs system;
          pkgs-stable = (mk-pkgs-stable system);
        };
      };
      machines = {
        framework = {
          system = "x86_64-linux";
          additionalModules = [
            nixos-hardware.nixosModules.framework-13-7040-amd
            ./hosts/framework/configuration.nix
          ];
          host = ./hosts/framework/host-cfg.nix;
        };
        desktop = {
          system = "x86_64-linux";
          additionalModules = [
            ./hosts/desktop/configuration.nix
          ];
          host = ./hosts/desktop/host-cfg.nix;
        };
      };
    in
    {
      nixosConfigurations = lib.mapAttrs (
        name: machine:
        let
          pkgs = pkgs-map.${machine.system};
          host = import machine.host { inherit pkgs; };
          ctx = { inherit pkgs host; };
        in
        lib.nixosSystem (
          (base-system machine.system)
          // {
            modules = (base-modules ctx) ++ machine.additionalModules;
          }
        )
      ) machines;

      packages = eachSystem (pkgs: {
        homeConfigurations = import ./nix/home-manager-standalone.nix {
          inherit pkgs inputs lib;
          inherit machines mk-pkgs-stable;
        };
      });

      formatter = eachSystem (pkgs: treefmtEval.${pkgs.system}.config.build.wrapper);

      devShells = eachSystem (pkgs: {
        default =
          let
            quickshell = inputs.quickshell.packages.${pkgs.system}.default;

            qml2_import = lib.concatStringsSep ":" [
              "${quickshell}/lib/qt-6/qml"
              "${pkgs.kdePackages.qtdeclarative}/lib/qt-6/qml"
              "${pkgs.kdePackages.kirigami.unwrapped}/lib/qt-6/qml"
            ];
          in
          pkgs.mkShell {
            nativeBuildInputs =
              [
                treefmtEval.${pkgs.system}.config.build.wrapper
              ]
              ++ (with pkgs; [
                nixd
                nixfmt-rfc-style
                nil

                lua-language-server

                kdePackages.qtdeclarative # qmlls
                quickshell

                nix-tree
              ]);

            shellHook = ''
              export ROOT_NIXOS_PATH=$(git rev-parse --show-toplevel)
              export QML2_IMPORT_PATH=${qml2_import}:$QML2_IMPORT_PATH
            '';
          };
      });
    };
}
