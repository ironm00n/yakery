{
  config,
  lib,
  my-lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf mkMerge;
  inherit (my-lib) mkDisableOption;
  cfg = config.bundles.virtualisation;
in
{
  options.bundles.virtualisation = {
    enable = mkEnableOption "virtualisation";
    qemu = mkDisableOption "qemu";
    libvirt = mkEnableOption "libvirt.";
    docker = mkEnableOption "Docker";
    waydroid = mkEnableOption "Waydroid";
    virtualbox = mkEnableOption "VirtualBox";
  };

  config = mkIf cfg.enable (mkMerge [
    (mkIf cfg.qemu {
      environment.systemPackages = with pkgs; [ qemu ];
    })
    (mkIf cfg.libvirt {
      virtualisation.libvirtd.enable = true;
      virtualisation.libvirtd.qemu.swtpm.enable = true;
      programs.virt-manager.enable = true;
    })
    (mkIf cfg.docker {
      virtualisation.docker.enable = true;
      environment.systemPackages = with pkgs; [ docker ];
    })
    (mkIf cfg.waydroid {
      # Waydroid is not declarative.
      # Useful docs:
      #   - https://docs.waydro.id/faq/google-play-certification
      #   - https://docs.waydro.id/faq/disable-on-screen-keyboard
      # Setup:
      #   `sudo waydroid init`; for google apps: `sudo waydroid init -s GAPPS -f`
      # Start:
      #   `sudo systemctl start waydroid-container`
      #   `waydroid session start`
      # Update Android:
      #   `sudo waydroid upgrade`
      # Usage:
      #   - `waydroid show-full-ui`
      virtualisation.waydroid.enable = true;
      virtualisation.waydroid.package = pkgs.waydroid-nftables; # needed for newer kernels
      environment.systemPackages = with pkgs; [ waydroid-helper cage ];
    })
    (mkIf cfg.virtualbox {
      virtualisation.virtualbox.host.enable = true;
      users.extraGroups.vboxusers.members = [ "ironmoon" ];
    })
  ]);
}
