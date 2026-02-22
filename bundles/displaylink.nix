{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.displaylink;
in
{
  options.bundles.displaylink = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable display link drivers.";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [
      pkgs.displaylink
    ];

    systemd.services.dlm.wantedBy = [ "multi-user.target" ];
  };
}
