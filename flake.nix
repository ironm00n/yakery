{
  description = "ironmoon's NixOS configuration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable-small";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.11";
    nixpkgs-25_05.url = "github:nixos/nixpkgs/nixos-25.05"; # twemoji-cbdt
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
    quickshell = {
      url = "git+https://git.outfoxxed.me/quickshell/quickshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixvim = {
      url = "github:nix-community/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
    };
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # NOTE: the following have their own instance of nixpkgs since their
    # dependencies are quite finicky
    binary-ninja = {
      url = "github:jchv/nix-binary-ninja";
      inputs.flake-utils.follows = "flake-utils";
    };
    pwndbg = {
      url = "github:pwndbg/pwndbg";
    };
  };

  nixConfig = {
    extra-substituters = [
      "https://ironmoon.cachix.org"
      "https://numtide.cachix.org"
      "https://hyprland.cachix.org"
      "https://nix-community.cachix.org"
    ];
    extra-trusted-public-keys = [
      "ironmoon.cachix.org-1:wowGL4TAzZPBO0fCqOekQLFqim3iXzdR+hIrK/tUadI="
      "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      "hyprland.cachix.org-1:a7pgxzMz7+chwVL3/pzj6jIBMioiJM7ypFP8PwtkuGc="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
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
      systems,
      flake-utils,
      nixvim,
      disko,
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
      eachSystem =
        f:
        lib.genAttrs all-systems (
          system:
          f {
            inherit system;
            pkgs = pkgs-map.${system};
          }
        );
      treefmtEval = eachSystem ({ pkgs, ... }: treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix);
      mk-pkgs-stable =
        system:
        import nixpkgs-stable {
          inherit system;
          config = base-nixpkgs-config;
        };
    in
    let
      use-lix = false;
      base-config =
        { pkgs, host }:
        {
          host = host;
          nixpkgs.pkgs = pkgs;

          nix.package = lib.mkIf use-lix pkgs.lix;
          nix.settings.experimental-features = [
            "nix-command"
            "flakes"
            "no-url-literals"
          ]
          ++ (if use-lix then [ "pipe-operator" ] else [ "pipe-operators" ]);
        };
      base-modules = ctx: [
        ./hosts/options.nix
        (base-config ctx)
        home-manager.nixosModules.home-manager
      ];
      base-system = system: {
        inherit system;
        specialArgs = {
          inherit inputs system;
          pkgs-stable = (mk-pkgs-stable system);
        };
      };
      machines = {
        fw12 = {
          system = "x86_64-linux";
          additionalModules = [
            nixos-hardware.nixosModules.framework-12-13th-gen-intel
            ./hosts/fw12/configuration.nix
          ];
          host = ./hosts/fw12/host-cfg.nix;
        };
        fw13 = {
          system = "x86_64-linux";
          additionalModules = [
            nixos-hardware.nixosModules.framework-13-7040-amd
            ./hosts/fw13/configuration.nix
          ];
          host = ./hosts/fw13/host-cfg.nix;
        };
        desktop = {
          system = "x86_64-linux";
          additionalModules = [
            ./hosts/desktop/configuration.nix
          ];
          host = ./hosts/desktop/host-cfg.nix;
        };
        hetzner-cx33-1 = {
          system = "x86_64-linux";
          additionalModules = [
            ./hosts/hetzner-cx33-1/configuration.nix
          ];
          host = ./hosts/hetzner-cx33-1/host-cfg.nix;
          no-hm = true;
        };
        ovh-vps1-1 = {
          system = "x86_64-linux";
          additionalModules = [
            disko.nixosModules.disko
            ./hosts/ovh-vps1-1/configuration.nix
          ];
          host = ./hosts/ovh-vps1-1/host-cfg.nix;
          no-hm = true;
        };
        oracle-e2-1-micro-1 = {
          system = "x86_64-linux";
          additionalModules = [
            disko.nixosModules.disko
            ./hosts/oracle-e2-1-micro-1/configuration.nix
          ];
          host = ./hosts/oracle-e2-1-micro-1/host-cfg.nix;
          no-hm = true;
        };
        oracle-e2-1-micro-2 = {
          system = "x86_64-linux";
          additionalModules = [
            disko.nixosModules.disko
            ./hosts/oracle-e2-1-micro-2/configuration.nix
          ];
          host = ./hosts/oracle-e2-1-micro-2/host-cfg.nix;
          no-hm = true;
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

      packages = eachSystem ({ system, pkgs }: {
        homeConfigurations = import ./nix/home-manager-standalone.nix {
          inherit pkgs inputs lib;
          inherit machines mk-pkgs-stable;
        };
        nvim = import ./nix/nvim/default.nix { inherit (nixvim.legacyPackages.${system}) makeNixvim; };
      });

      formatter = eachSystem ({ system, ... }: treefmtEval.${system}.config.build.wrapper);

      devShells = eachSystem ({ system, pkgs }: {
        default =
          let
            # TOOD: reenable when needed
            enable-quickshell = false;
            quickshell = inputs.quickshell.packages.${system}.default;

            qml2_import =
              lib.optional enable-quickshell [
                "${quickshell}/lib/qt-6/qml"
                "${pkgs.kdePackages.qtdeclarative}/lib/qt-6/qml"
                "${pkgs.kdePackages.kirigami.unwrapped}/lib/qt-6/qml"
              ]
              |> lib.concatStringsSep ":";
          in
          pkgs.mkShell {
            nativeBuildInputs = [
              treefmtEval.${system}.config.build.wrapper
            ]
            ++ (with pkgs; [
              nixd
              nixfmt
              nil

              lua-language-server

              nix-tree
            ])
            ++ lib.optionals enable-quickshell [
              pkgs.kdePackages.qtdeclarative # qmlls
              quickshell
            ];

            shellHook = ''
              export ROOT_NIXOS_PATH=$(git rev-parse --show-toplevel)
              export QML2_IMPORT_PATH=${qml2_import}:$QML2_IMPORT_PATH
            '';
          };
      });
    };
}
