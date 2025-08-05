{
  config,
  lib,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.gaming;
in
{
  options.bundles.gaming = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable gaming-specific system mofifications.";
    };
  };

  config = mkIf cfg.enable {
    programs.steam = {
      enable = true;
      remotePlay.openFirewall = true;
      dedicatedServer.openFirewall = true;
      localNetworkGameTransfers.openFirewall = true;
    };

    hardware.xone.enable = true;
  };
}
