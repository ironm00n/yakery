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
    ./disks.nix
    ../common/server.nix
  ];

  boot.loader.grub = {
    efiSupport = true;
    efiInstallAsRemovable = true;
  };

  networking = {
    interfaces.enp1s0.ipv6.addresses = [
      {
        address = "2604:2dc0:202:300::143f";
        prefixLength = 128;
      }
    ];
    defaultGateway6 = {
      address = "2604:2dc0:202:300::1";
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
