{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.eww;
in
{
  options.bundles.eww = {
    enable = mkEnableOption "eww bar";
  };

  config = mkIf cfg.enable {
    home.packages = with pkgs; [
      eww
      hyprland-workspaces
    ];

    xdg.configFile."eww/eww.yuck".source = symlink ./eww.yuck;
    xdg.configFile."eww/eww.scss".source = symlink ./eww.scss;
  };
}
