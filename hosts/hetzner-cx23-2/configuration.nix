{ ... }:
{
  imports = [
    ../common/hetzner-cloud
    ../common/hetzner-cloud/qemu-guest.nix
    ../common/disko-lvm-ext4.nix
    ../common/server
  ];

  system.stateVersion = "26.11";
}
