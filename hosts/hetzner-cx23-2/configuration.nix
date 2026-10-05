{ ... }:
{
  imports = [
    ../common/hetzner-cloud
    ../common/hetzner-cloud/qemu-guest.nix
    ../common/disko-lvm-ext4.nix
    ../common/server
    ../../modules/zitadel
  ];

  # TODO: swap the module everywhere once hetzner-cx33-1 stops running 2.71.7
  disabledModules = [ "services/web-apps/zitadel.nix" ];
  bundles.zitadel.enable = true;

  system.stateVersion = "26.11";
}
