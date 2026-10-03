{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf mkMerge;
  cfg = config.bundles.keychron;
in
{
  options.bundles.keychron = {
    enable = mkEnableOption "Keychron Launcher (WebHID) access to Keychron keyboards";
    flashing = mkEnableOption "QMK bootloader access, for flashing firmware from Keychron Launcher";
  };

  config = mkMerge [
    (mkIf cfg.enable { services.udev.packages = [ pkgs.keychron-udev-rules ]; })
    (mkIf cfg.flashing { hardware.keyboard.qmk.enable = true; })
  ];
}
