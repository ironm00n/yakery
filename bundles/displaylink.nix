{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.displaylink;
in
{
  options.bundles.displaylink = {
    enable = mkEnableOption "DisplayLink drivers";
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [
      pkgs.displaylink
    ];

    systemd.services.dlm.wantedBy = [ "multi-user.target" ];
  };
}
