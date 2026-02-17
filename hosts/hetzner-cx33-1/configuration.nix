# Edit this configuration file to define what should be installed on
# your system. Help is available in the configuration.nix(5) man page, on
# https://search.nixos.org/options and in the NixOS manual (`nixos-help`).

{
  config,
  lib,
  pkgs,
  ...
}:

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
          address = "2a01:4f8:c0c:1982::1";
          prefixLength = 64;
        }
      ];
      ipv4.addresses = [
        {
          address = "91.98.113.48";
          prefixLength = 32;
        }
      ];
    };
    defaultGateway6 = {
      address = "fe80::1";
      interface = "enp1s0";
    };
    defaultGateway = {
      address = "172.31.1.1";
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
