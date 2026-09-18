{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.displaylink;
  inherit (config.boot.kernelPackages) evdi;
  displaylink = pkgs.displaylink.override { inherit evdi; };
in
{
  options.bundles.displaylink = {
    enable = mkEnableOption "DisplayLink drivers";
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [
      displaylink
    ];

    boot = {
      extraModulePackages = [ evdi ];
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
        ExecStart = "${displaylink}/bin/DisplayLinkManager";
        User = "root";
        Group = "root";
        # Environment = [ "DISPLAY=:0" ];
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
