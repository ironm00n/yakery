{
  inputs,
  config,
  pkgs-stable,
  lib,
  pkgs,
  ...
}:
{
  home-manager = {
    backupFileExtension = ".bak";
    useUserPackages = true;
    useGlobalPkgs = true;
    sharedModules = [
      # needed even when not using fill kde (konsole, dolphin, etc)
      inputs.plasma-manager.homeManagerModules.plasma-manager
    ];
    extraSpecialArgs = {
      inherit pkgs-stable inputs;
      inherit (config) host;
      # TODO: how to get home-manager's version of config?
      my-utils = import ./my-utils.nix {
        inherit lib pkgs inputs;
        inherit (config) host;
        inherit (inputs.home-manager.lib) hm;
      };
    };
    users.ironmoon = ./ironmoon/home-manager.nix;
  };
}
