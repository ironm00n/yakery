{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.ok-color;
  colors = [
    "black"
    "red"
    "green"
    "yellow"
    "blue"
    "magenta"
    "cyan"
    "white"
  ];
in
{
  options.bundles.ok-color = {
    enable = mkEnableOption "picking the color of systemd's `[  OK  ]` boot status";
    color = mkOption {
      type = types.enum (colors ++ map (c: "highlight-${c}") colors);
      default = "highlight-blue";
      description = "systemd's `ok-color` meson option.";
    };
  };

  config = mkIf cfg.enable {
    systemd.package = pkgs.systemd.overrideAttrs (old: {
      mesonFlags = old.mesonFlags ++ [ (lib.mesonOption "ok-color" cfg.color) ];
    });
  };
}
