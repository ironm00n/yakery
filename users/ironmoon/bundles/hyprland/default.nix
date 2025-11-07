{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.hyprland;
in
{
  options.bundles.hyprland = {
    enable = mkEnableOption "hyprland";
  };

  config = mkIf cfg.enable {
    wayland.windowManager.hyprland = import ./config.nix { inherit config lib pkgs; };

    xdg.configFile."hypr/xdph.conf".text = lib.hm.generators.toHyprconf {
      attrs = import ./portal.nix { inherit pkgs; };
    };
  };
}
