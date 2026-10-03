# manually formatted
{
  description = "ironmoon's NixOS configuration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-26.05";
    # this can't be replaced by multiverse since it only indexes once something lands on unstable
    nixpkgs-master.url = "github:NixOS/nixpkgs/master";
    multiverse.url = "github:fzakaria/nixpkgs-multiverse";
    systems.url = "github:nix-systems/default";
    nixos-hardware = {
      url = "github:NixOS/nixos-hardware/master";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sops-nix = {
      url = "github:Mic92/sops-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    secrets = {
      # private repo exporting sops files
      url = "git+ssh://git@github.com/ironm00n/secrets.git";
      inputs.nixpkgs.follows = "";
      inputs.systems.follows = "";
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
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    quickshell = {
      url = "git+https://git.outfoxxed.me/quickshell/quickshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
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
    deploy-rs = {
      url = "github:serokell/deploy-rs";
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
    # its nixConfig substituters are deliberately not trusted; built locally
    codex-desktop-linux = {
      url = "github:ilysenko/codex-desktop-linux";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
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

  outputs = inputs@{
    self,
    nixpkgs,
    nixpkgs-stable,
    nixpkgs-master,
    multiverse,
    systems,
    nixos-hardware,
    sops-nix,
    home-manager,
    plasma-manager,
    treefmt-nix,
    nixvim,
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
    mk-pkgs-map = np:
      all-systems
      |> map (system: {
        name = system;
        value = import np {
          inherit system overlays;
          config = base-nixpkgs-config // {
            permittedInsecurePackages = [
              "electron-39.8.10"
            ];
          };
        };
      })
      |> builtins.listToAttrs;
    # need to call here to for nix to memoize
    pkgs-map = mk-pkgs-map nixpkgs;
    pkgs-map-master = mk-pkgs-map nixpkgs-master;
    eachSystem = f:
      lib.genAttrs all-systems (
        system:
        f {
          inherit system;
          pkgs = pkgs-map.${system};
        }
      );
    treefmtEval = eachSystem ({ pkgs, ... }: treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix);
    mk-pkgs-stable = system:
      import nixpkgs-stable {
        inherit system;
        config = base-nixpkgs-config;
      };
    mk-mv = system:
      multiverse.lib.mkMultiverse {
        inherit system;
        config = base-nixpkgs-config;
        overlays = [];
      };
    use-lix = false;
    base-config = { pkgs, host, ... }: {
      host = host;
      nixpkgs.pkgs = pkgs;
      nix.package = lib.mkIf use-lix pkgs.lix;
      nix.settings.lint-url-literals = "fatal";
      nix.settings.experimental-features = [
        "nix-command"
        "flakes"
      ] ++ (if use-lix then [ "pipe-operator" ] else [ "pipe-operators" ]);
    };
    base-modules = ctx:
      let
        use-hm = !(ctx.machine.no-hm or false);
        use-secrets = !(ctx.machine.no-secrets or false);
      in
      [
        ./hosts/options.nix
        (base-config ctx)
        { host.use-secrets = use-secrets; }
      ] ++ lib.optionals use-secrets [
        sops-nix.nixosModules.sops
      ] ++ lib.optionals use-hm [
        home-manager.nixosModules.home-manager
      ];
    my-lib = import ./lib { inherit lib; };
    base-system = system: {
      inherit system;
      specialArgs = {
        inherit inputs system my-lib;
        mv = (mk-mv system);
        pkgs-stable = (mk-pkgs-stable system);
        pkgs-master = pkgs-map-master.${system};
      };
    };
    mk-server = { id, system, disko ? false }: {
      ${id} = {
        inherit system;
        additionalModules = [
          (./hosts + "/${id}" + "/configuration.nix")
        ] ++ lib.optionals disko [
          inputs.disko.nixosModules.disko
        ];
        host = {pkgs}: {
          inherit id;
          hostname = id;
        };
        no-hm = true;
      };
    };
    servers =
      (mk-server { id = "hetzner-cx33-1"; system = "x86_64-linux"; })
      // (mk-server { id = "hetzner-cx23-1"; system = "x86_64-linux"; disko = true; })
      // (mk-server { id = "hetzner-cx23-2"; system = "x86_64-linux"; disko = true; })
      // (mk-server { id = "ovh-vps1-1"; system = "x86_64-linux"; disko = true; })
      // (mk-server { id = "oracle-e2-1-micro-1"; system = "x86_64-linux"; disko = true; })
      // (mk-server { id = "oracle-e2-1-micro-2"; system = "x86_64-linux"; disko = true; })
      # // (mk-server { id = "oracle-e2-1-micro-3"; system = "x86_64-linux"; disko = true; })
      # // (mk-server { id = "oracle-e2-1-micro-4"; system = "x86_64-linux"; disko = true; })
      // (mk-server { id = "oracle-a1-flex-1"; system = "aarch64-linux"; disko = true; })
      // (mk-server { id = "oracle-a1-flex-2"; system = "aarch64-linux"; disko = true; })
      // (mk-server { id = "oracle-a1-flex-3"; system = "aarch64-linux"; disko = true; })
      // (mk-server { id = "pi5"; system = "aarch64-linux"; })
        ;
    machines = {
      fw12 = {
        system = "x86_64-linux";
        additionalModules = [
          nixos-hardware.nixosModules.framework-12-13th-gen-intel
          ./hosts/fw12/configuration.nix
        ];
        host = import ./hosts/fw12/host-cfg.nix;
      };
      fw13 = {
        system = "x86_64-linux";
        additionalModules = [
          nixos-hardware.nixosModules.framework-13-7040-amd
          ./hosts/fw13/configuration.nix
        ];
        host = import ./hosts/fw13/host-cfg.nix;
      };
      desktop = {
        system = "x86_64-linux";
        additionalModules = [
          ./hosts/desktop/configuration.nix
        ];
        host = import ./hosts/desktop/host-cfg.nix;
      };
    }
    // servers;
  in
  {
    nixosConfigurations = lib.mapAttrs (
      name: machine:
      let
        pkgs = pkgs-map.${machine.system};
        host = machine.host { inherit pkgs; };
        ctx = { inherit pkgs host machine; };
      in
      lib.nixosSystem (
        (base-system machine.system)
        // {
          modules = (base-modules ctx) ++ machine.additionalModules;
        }
      )
    ) machines;

    deploy.nodes = lib.mapAttrs (
      id: machine:
      let
        ip = inputs.secrets.data.ips.${id} or { };
        hostname =
          if ip ? ipv4.address then ip.ipv4.address
          else if ip ? ipv6.address then ip.ipv6.address
          else id;
      in
      {
        inherit hostname;
        sshUser = "root";
        profiles.system.path =
          inputs.deploy-rs.lib.${machine.system}.activate.nixos
            self.nixosConfigurations.${id};
      }
    ) servers;

    checks = lib.mapAttrs (
      system: deployLib:
      deployLib.deployChecks self.deploy
      // lib.optionalAttrs (system == "x86_64-linux") {
        oom = import ./nix/tests/oom.nix { pkgs = pkgs-map.${system}; };
        ksycoca = import ./nix/tests/ksycoca.nix { pkgs = pkgs-map.${system}; };
        ok-color = import ./nix/tests/ok-color.nix { pkgs = pkgs-map.${system}; };
        borg-hop = import ./nix/tests/borg-hop.nix { pkgs = pkgs-map.${system}; };
      }
    ) inputs.deploy-rs.lib;

    homeConfigurations = eachSystem ({ system, pkgs }:
      import ./nix/home-manager-standalone.nix {
        inherit pkgs inputs lib my-lib;
        inherit machines mk-mv mk-pkgs-stable;
        pkgs-master = pkgs-map-master.${system};
      });

    packages = eachSystem ({ system, pkgs }: {
      nvim = import ./nix/nvim/default.nix {
        inherit (nixvim.legacyPackages.${system}) makeNixvimWithModule;
        pkgs = pkgs-map.${system};
      };
    }
    // lib.optionalAttrs (system == "x86_64-linux") {
      # Throwaway guest for untrusted coding harnesses; not a `machines` entry on purpose.
      hackathon-vm =
        (import ./nix/vms/hackathon-vm.nix { inherit inputs system; })
        .config.system.build.sandbox;
      hackathon-vm-clip-push = (import ./nix/vms/hackathon-vm-clip.nix { inherit pkgs; }).push;
      hackathon-vm-clip-pull = (import ./nix/vms/hackathon-vm-clip.nix { inherit pkgs; }).pull;
    });

    formatter = eachSystem ({ system, ... }: treefmtEval.${system}.config.build.wrapper);

    devShells = eachSystem ({ system, pkgs }: {
      default =
        let
          # TODO: reenable when needed
          enable-quickshell = false;
          quickshell = inputs.quickshell.packages.${system}.default;
          qml2_import =
            lib.optional enable-quickshell [
              "${quickshell}/lib/qt-6/qml"
              "${pkgs.kdePackages.qtdeclarative}/lib/qt-6/qml"
              "${pkgs.kdePackages.kirigami.unwrapped}/lib/qt-6/qml"
            ]
            |> lib.concatStringsSep ":";
          # lua-language-server expands a leading $VAR in workspace.library, so the
          # hyprland bundle's .luarc.json reaches the `hl` stubs through this export
          hyprland_lua_stubs =
            let
              var = "HYPRLAND_LUA_STUBS";
              ref = "$" + var;
              luarc-path = "users/ironmoon/bundles/hyprland/.luarc.json";
              luarc = builtins.fromJSON (builtins.readFile (./. + "/${luarc-path}"));
            in
            assert lib.assertMsg (builtins.elem ref luarc.workspace.library)
              "${luarc-path}: workspace.library must reference ${ref}";
            lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              export ${var}=${pkgs.hyprland}/share/hypr/stubs
            '';
        in
        pkgs.mkShell {
          nativeBuildInputs = [
            treefmtEval.${system}.config.build.wrapper
            inputs.deploy-rs.packages.${system}.default
          ] ++ (with pkgs; [
            nixd
            nixfmt
            nil
            lua-language-server
            nix-tree
            cargo
          ]) ++ lib.optionals enable-quickshell [
            pkgs.kdePackages.qtdeclarative # qmlls
            quickshell
          ];
          shellHook = ''
            export ROOT_NIXOS_PATH=$(git rev-parse --show-toplevel)
            export QML2_IMPORT_PATH=${qml2_import}:$QML2_IMPORT_PATH
            ${hyprland_lua_stubs}
          '';
        };
    });
  };
}
