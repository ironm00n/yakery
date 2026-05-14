{
  config,
  lib,
  ...
}@args:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.waybar;
in
{
  options.bundles.waybar = {
    enable = mkEnableOption "waybar";
  };

  config = mkIf cfg.enable {
    programs.waybar = import ./program.nix args;
  };
}
