{
  pkgs,
  inputs,
  lib,
  my-lib,
  machines,
  mk-pkgs-stable,
}:
let
  inherit (inputs) home-manager;
  pkgs-stable = mk-pkgs-stable pkgs.stdenv.hostPlatform.system;

  # FIXME: figure out how to play nicely with specializations
  # fornow, assume we are using hyprland
  host =
    machine:
    let
      raw = (machines.${machine}.host { inherit pkgs; }) // { hyprland = true; };
      use-secrets = !(machines.${machine}.no-secrets or false);
    in
    (lib.evalModules {
      modules = [
        ../hosts/options.nix
        { host = raw // { inherit use-secrets; }; }
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
        inherit pkgs pkgs-stable;
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
in
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
|> lib.listToAttrs
