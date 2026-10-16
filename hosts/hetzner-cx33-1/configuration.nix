# Edit this configuration file to define what should be installed on
# your system. Help is available in the configuration.nix(5) man page, on
# https://search.nixos.org/options and in the NixOS manual (`nixos-help`).

{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  ...
}:
{
  imports = [
    ./hardware-configuration.nix
    ../common/server
    ../common/hetzner-cloud
  ];

  bundles.stalwart = {
    enable = true;
    # agent-dispatch on desktop wakes the agents a delivery is addressed to
    deliveryWebhooks = [ "http://${my-lib.netbird.peers.desktop.v4}:7310/" ];
    receiving.enable = true;
  };
  bundles.bulwark.enable = true;
  bundles.vpn.netbird = {
    enable = true;
    useSetupKey = true;
  };

  boot.loader.grub = {
    enable = true;
    device = "/dev/sda";
  };

  system.stateVersion = "25.05";
}
