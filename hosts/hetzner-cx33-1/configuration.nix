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
  inherit (inputs.secrets.data.ips.hetzner-cx33-1) ipv4 ipv6;
in
{
  imports = [
    ./hardware-configuration.nix
    ../common/server.nix
    ./zitadel.nix
  ];

  boot.loader.grub = {
    enable = true;
    device = "/dev/sda";
  };

  networking = {
    interfaces.enp1s0 = {
      ipv6.addresses = [
        {
          address = "${ipv6.prefix}::1";
          inherit (ipv6) prefixLength;
        }
      ];
      ipv4.addresses = [
        {
          inherit (ipv4) address prefixLength;
        }
      ];
    };
    defaultGateway6 = {
      address = ipv6.gateway;
      interface = "enp1s0";
    };
    defaultGateway = {
      address = ipv4.gateway;
      interface = "enp1s0";
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
