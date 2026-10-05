{ ... }:
{
  imports = [
    ../common/hetzner-cloud
    ../common/hetzner-cloud/qemu-guest.nix
    ../common/disko-lvm-ext4.nix
    ../common/server
  ];

  bundles.zitadel.enable = true;

  system.stateVersion = "26.11";
}
