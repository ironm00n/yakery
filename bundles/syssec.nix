{
  config,
  lib,
  pkgs,
  inputs,
  system,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.syssec;
  inherit (inputs) binary-ninja pwndbg;
in
{
  options.bundles.syssec = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "System Security tools.";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = with pkgs; [
      gef

      binary-ninja.packages.${system}.binary-ninja-free-wayland
      pwndbg.packages.${system}.pwndbg
    ];
  };
}
