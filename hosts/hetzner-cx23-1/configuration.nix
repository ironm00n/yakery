{ config, ... }:
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
    target = {
      host = "u668784.your-storagebox.de";
      user = "u668784";
      port = 23;
      hostPublicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIICf9svRenC/PLKIL9nk6K/pxQgoiFC41wTNvoIncOxs";
      remotePath = "borg-1.4";
      repositories = [
        "/home/desktop"
        "/home/archive-sandisk"
        "/home/archive-arch"
      ];
    };
  };

  system.stateVersion = "26.05";
}
