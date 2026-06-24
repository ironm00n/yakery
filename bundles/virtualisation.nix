{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.virtualisation;
in
{
  options.bundles.virtualisation = {
    enable = mkEnableOption "virtualisation";
    libvirt = mkEnableOption "libvirt.";
    docker = mkEnableOption "Docker";
    waydroid = mkEnableOption "Waydroid";
    virtualbox = mkEnableOption "VirtualBox";
  };

  config = mkIf cfg.enable {
    virtualisation.libvirtd.enable = cfg.libvirt;
    virtualisation.libvirtd.qemu.swtpm.enable = cfg.libvirt;
    programs.virt-manager.enable = cfg.libvirt;

    virtualisation.docker.enable = cfg.docker;

    virtualisation.waydroid.enable = cfg.waydroid;

    environment.systemPackages =
      (with pkgs; [ qemu ]) ++ (lib.optionals cfg.docker (with pkgs; [ docker ]));

    virtualisation.virtualbox.host.enable = cfg.virtualbox;
    users.extraGroups.vboxusers.members = mkIf cfg.virtualbox [ "ironmoon" ];
  };
}
