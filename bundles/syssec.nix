{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.syssec;
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
    programs.binary-ninja = {
      enable = true;
      package = pkgs.binary-ninja-free-wayland;
    };

    environment.systemPackages = with pkgs; [
      gef
      # pwndbg
    ];
  };
}
