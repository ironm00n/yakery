{
  config,
  lib,
  pkgs,
  system,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.harden;
in
{
  options.bundles.harden = {
    enable = mkEnableOption "system hardening";
  };

  config = mkIf cfg.enable {

  };
}
