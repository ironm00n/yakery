{
  inputs,
  config,
  pkgs-stable,
  lib,
  my-lib,
  pkgs,
  pkgs-master,
  mv,
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
      inherit inputs lib my-lib;
      inherit mv;
      inherit (config) host;
      inherit pkgs pkgs-stable pkgs-master;
    };
    users.ironmoon = ./ironmoon/home-manager.nix;
  };
}
