{
  inputs,
  config,
  pkgs-stable,
  lib,
  my-lib,
  pkgs,
  ...
}:
{
  home-manager = lib.mkIf config.host.home-manager-nixos {
    backupFileExtension = ".bak";
    useUserPackages = true;
    useGlobalPkgs = true;
    sharedModules = import ./home-shared-modules.nix {
      inherit inputs lib;
      useSecrets = config.host.use-secrets;
    };
    extraSpecialArgs = import ./extra-special-args.nix {
      inherit inputs lib;
      inherit pkgs pkgs-stable my-lib;
      inherit (config) host;
    };
    users.ironmoon = ./ironmoon/home-manager.nix;
  };
}
