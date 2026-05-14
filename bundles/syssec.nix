{
  config,
  lib,
  pkgs,
  inputs,
  system,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.syssec;
  inherit (inputs) binary-ninja pwndbg;
in
{
  options.bundles.syssec = {
    enable = mkEnableOption "system Security tools";
  };

  config = mkIf cfg.enable {
    environment.systemPackages = with pkgs; [
      gef

      binary-ninja.packages.${system}.binary-ninja-free-wayland
      pwndbg.packages.${system}.pwndbg
    ];
  };
}
