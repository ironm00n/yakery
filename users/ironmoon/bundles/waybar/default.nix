{
  config,
  lib,
  ...
}@args:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.waybar;
in
{
  options.bundles.waybar = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable waybar.";
    };
  };

  config = mkIf cfg.enable {
    programs.waybar = import ./program.nix args;
  };
}
