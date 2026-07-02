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

    boot = {
      extraModulePackages = [ config.boot.kernelPackages.evdi ];
      initrd = {
        kernelModules = [
          "evdi"
        ];
      };
    };

    systemd.services.displaylink-server = {
      enable = true;
      requires = [ "systemd-udevd.service" ];
      after = [ "systemd-udevd.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.displaylink}/bin/DisplayLinkManager";
        User = "root";
        Group = "root";
        # Environment = [ "DISPLAY=:0" ];
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
