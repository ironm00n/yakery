{ config, my-lib, ... }:
{
  imports = [
    ../common/hetzner-cloud/qemu-guest.nix
    ../common/disko-lvm-ext4.nix
    ../common/server
  ];

  networking.nameservers = [
    "2606:4700:4700::1111"
    "2606:4700:4700::1001"
    "1.1.1.1"
    "1.0.0.1"
  ];

  bundles.vpn.netbird.enable = true;

  bundles.borg-hop = {
    enable = true;
    interfaces = [ config.services.netbird.clients.netbird.interface ];
    target = my-lib.borg.storagebox // {
      repositories = [
        "/home/desktop"
        "/home/archive-sandisk"
        "/home/archive-arch"
        "/home/archive-windows"
      ];
    };
  };

  system.stateVersion = "26.05";
}
