{
  config,
  lib,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.syncthing;
in
{
  options.bundles.syncthing = {
    enable = mkEnableOption "syncthing";
  };

  config = mkIf cfg.enable {
    services.syncthing = {
      enable = true;
      overrideDevices = false;
      overrideFolders = false;
      settings = {

        folders = {
          "/home/${config.home.username}/documents" = {
            id = "documents";
            label = "documents";
            devices = [ "desktop" "fw13" "fw12" ];
          };
        };

        devices = {
          desktop = {
            id = "ML7KRVX-JUHEMVT-MXLN6DR-GSVNVEG-MSFZEKA-7IPVFON-EZYZI6R-4SGCZQM";
            addresses = [ "tcp://100.69.0.1" ];
          };

          fw13 = {
            id = "MU3S3QL-ILFFKVR-USGFR2Y-UCLAIO2-AABFHHA-RL32JNC-U55UKHE-XY2CTQ2";
            addresses = [ "tcp://100.69.0.2" ];
          };

          fw12 = {
            id = "U65572Q-3X3PZ7N-KCWR44D-LWAF2RK-P3PN6WG-LZ5HGDA-27RUGS2-BOO4SA3";
            addresses = [ "tcp://100.69.0.3" ];
          };
        };

        options = {
          # we want our netbird vpn to handle all of this kinda stuff
          globalAnnounceEnabled = false;
          localAnnounceEnabled = false;
          natEnabled = false;
          stunKeepaliveStartS = 0;

          relaysEnabled = false;

          # telemtry
          urAccepted = -1;
          crashReportingEnabled = false;

          autoUpgradeIntervalH = 0;
        };
      };
    };
  };
}
