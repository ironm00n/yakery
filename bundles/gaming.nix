{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.gaming;
in
{
  options.bundles.gaming = {
    enable = mkEnableOption "gaming";
    vr = mkEnableOption "vr";
  };

  config = mkIf cfg.enable {
    programs.steam = {
      enable = true;
      remotePlay.openFirewall = true;
      dedicatedServer.openFirewall = true;
      localNetworkGameTransfers.openFirewall = true;
    };

    hardware.xone.enable = true;

    environment.systemPackages =
      with pkgs;
      [ ]
      ++ (lib.optionals cfg.vr [
        bs-manager
      ]);
  };
}
