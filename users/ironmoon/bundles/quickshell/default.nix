{
  config,
  lib,
  pkgs,
  my-utils,
  inputs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.quickshell;
  quickshellDir = "${config.xdg.configHome}/quickshell";
in
{
  options.bundles.quickshell = {
    enable = mkEnableOption "Quickshell";
  };

  config = mkIf cfg.enable {
    home.packages = [
      inputs.quickshell.packages.${pkgs.system}.default
    ];

    home.file."${quickshellDir}/bar".source = symlink ./bar;
    home.file."${quickshellDir}/activate-linux".source = symlink ./activate-linux;
  };
}
