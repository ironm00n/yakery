{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.eww;
  ewwDir = "${config.xdg.configHome}/eww";
in
{
  options.bundles.eww = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable eww bar.";
    };
  };

  config = mkIf cfg.enable {
    home.packages = with pkgs; [ eww hyprland-workspaces ];

    home.file."${ewwDir}/eww.yuck".source = symlink ./eww.yuck;
    home.file."${ewwDir}/eww.scss".source = symlink ./eww.scss;
  };
}
