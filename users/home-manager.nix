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
    sharedModules = [
      inputs.nixvim.homeModules.nixvim
      # needed even when not using full kde (konsole, dolphin, etc)
      inputs.plasma-manager.homeModules.plasma-manager
    ]
    ++ lib.optionals config.host.use-secrets [
      inputs.sops-nix.homeManagerModules.sops
    ];
    extraSpecialArgs = import ./extra-special-args.nix {
      inherit inputs lib;
      inherit pkgs pkgs-stable my-lib;
      inherit (config) host;
    };
    users.ironmoon = ./ironmoon/home-manager.nix;
  };
}
