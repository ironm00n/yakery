{
  pkgs,
  inputs,
  lib,
  machines,
  mk-pkgs-stable,
}:
let
  inherit (inputs) home-manager plasma-manager;
  pkgs-stable = mk-pkgs-stable pkgs.system;

  # FIXME: figure out how to play nicely with specializations
  # fornow, assume we are using hyprland
  host = machine: (import machines.${machine}.host { inherit pkgs; }) // { hyprland = true; };
  common =
    { username, machine }:
    {
      inherit pkgs;
      extraSpecialArgs = import ../users/extra-special-args.nix {
        inherit inputs lib;
        inherit pkgs pkgs-stable;
        host = host machine;
      };
      modules = [
        {
          home.username = username;
          home.homeDirectory = "/home/${username}";
        }
        plasma-manager.homeModules.plasma-manager
      ];
    };
  users = {
    ironmoon = [ ../users/ironmoon/home-manager.nix ];
  };
in
lib.cartesianProduct {
  username = lib.attrNames users;
  machine = machines |> builtins.filter (m: not (m.no-hm or false)) |> lib.attrNames;
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
