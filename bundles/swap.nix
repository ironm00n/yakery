{ config, lib, ... }:
let
  inherit (lib)
    mkEnableOption
    mkOption
    mkIf
    types
    ;
  cfg = config.bundles.swap;
in
{
  options.bundles.swap = {
    enable = mkEnableOption "a swapfile at /swapfile";
    size = mkOption {
      type = types.ints.positive;
      description = "The swapfile's size, in MiB.";
    };
  };

  config = mkIf cfg.enable {
    swapDevices = [
      {
        device = "/swapfile";
        inherit (cfg) size;
      }
    ];
  };
}
