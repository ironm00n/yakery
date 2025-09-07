{
  inputs,
  config,
  pkgs-stable,
  lib,
  pkgs,
  ...
}:
{
  home-manager = lib.mkIf config.host.home-manager-nixos {
    backupFileExtension = ".bak";
    useUserPackages = true;
    useGlobalPkgs = true;
    sharedModules = [
      # needed even when not using full kde (konsole, dolphin, etc)
      inputs.plasma-manager.homeModules.plasma-manager
    ];
    extraSpecialArgs = import ./extra-special-args.nix {
      inherit inputs lib;
      inherit pkgs pkgs-stable;
      inherit (config) host;
    };
    users.ironmoon = ./ironmoon/home-manager.nix;
  };
}
