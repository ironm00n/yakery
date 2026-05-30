# Edit this configuration file to define what should be installed on
# your system. Help is available in the configuration.nix(5) man page, on
# https://search.nixos.org/options and in the NixOS manual (`nixos-help`).

{
  config,
  lib,
  pkgs,
  inputs,
  ...
}:
let
  inherit (inputs.secrets.data.ips.ovh-vps1-1) ipv6;
in
{
  imports = [
    ./hardware-configuration.nix
    ./disks.nix
    ../common/server.nix
    ./netbird.nix
  ];

  boot.loader.grub = {
    efiSupport = true;
    efiInstallAsRemovable = true;
  };

  networking = {
    interfaces.ens3.ipv6.addresses = [
      {
        inherit (ipv6) address prefixLength;
      }
    ];
    defaultGateway6 = {
      address = ipv6.gateway;
      interface = "ens3";
    };
    nameservers = [
      "2606:4700:4700::1111"
      "2606:4700:4700::1001"
      "1.1.1.1"
      "1.0.0.1"
    ];
  };

  system.stateVersion = "25.05";
}
