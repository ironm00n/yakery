{
  pkgs,
  inputs,
  lib,
  my-lib,
  machines,
  mk-mv,
  mk-pkgs-stable,
  pkgs-master,
}:
let
  inherit (inputs) home-manager;
  mv = (mk-mv pkgs.stdenv.hostPlatform.system);
  pkgs-stable = mk-pkgs-stable pkgs.stdenv.hostPlatform.system;

  # FIXME: figure out how to play nicely with specializations
  # fornow, assume we are using hyprland
  host =
    machine:
    let
      raw = (machines.${machine}.host { inherit pkgs; }) // {
        hyprland = true;
      };
      use-secrets = !(machines.${machine}.no-secrets or false);
    in
    (lib.evalModules {
      modules = [
        ../hosts/options.nix
        {
          host = raw // {
            inherit use-secrets;
          };
        }
      ];
    }).config.host;
  common =
    { username, machine }:
    let
      hostCfg = host machine;
    in
    {
      inherit pkgs;
      extraSpecialArgs = import ../users/extra-special-args.nix {
        inherit inputs lib my-lib;
        inherit mv;
        inherit pkgs pkgs-stable pkgs-master;
        host = hostCfg;
      };
      modules = [
        {
          home.username = username;
          home.homeDirectory = "/home/${username}";
        }
      ]
      ++ import ../users/home-shared-modules.nix {
        inherit inputs lib;
        useSecrets = hostCfg.use-secrets;
      };
    };
  users = {
    ironmoon = [ ../users/ironmoon/home-manager.nix ];
  };
  # each account agents run as, on each machine whose system has them (`agents` in flake.nix's machines), deployed by
  # the account itself, so its home needs none of his modules
  agents =
    lib.cartesianProduct {
      account = lib.attrValues (import ../users/agent/accounts.nix);
      machine = machines |> lib.filterAttrs (_: m: m.agents or false) |> lib.attrNames;
    }
    |> map (
      { account, machine }:
      {
        name = "${account.name}@${machine}";
        value = home-manager.lib.homeManagerConfiguration {
          inherit pkgs;
          inherit
            (common {
              username = account.name;
              inherit machine;
            })
            extraSpecialArgs
            ;
          modules = [ (import ../users/agent/home account) ];
        };
      }
    )
    |> lib.listToAttrs;
  homes =
    lib.cartesianProduct {
      username = lib.attrNames users;
      machine = machines |> lib.filterAttrs (_: m: !(m.no-hm or false)) |> lib.attrNames;
    }
    |> map (
      { username, machine }:
      {
        name = "${username}@${machine}";
        value = home-manager.lib.homeManagerConfiguration (
          let
            base = common { inherit username machine; };
          in
          {
            inherit pkgs;
            inherit (base) extraSpecialArgs;
            modules = base.modules ++ users.${username};
          }
        );
      }
    )
    |> lib.listToAttrs;
in
homes // agents
