{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.virtualisation;
in
{
  options.bundles.virtualisation = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable virtualisation.";
    };

    libvirt = mkOption {
      type = types.bool;
      default = false;
      description = "Enable libvirt.";
    };

    docker = mkOption {
      type = types.bool;
      default = false;
      description = "Enable Docker.";
    };

    waydroid = mkOption {
      type = types.bool;
      default = false;
      description = "Enable Waydroid.";
    };

    virtualbox = mkOption {
      type = types.bool;
      default = false;
      description = "Enable virtualbox.";
    };
  };

  config = mkIf cfg.enable {
    virtualisation.libvirtd.enable = cfg.libvirt;
    virtualisation.libvirtd.qemu.swtpm.enable = cfg.libvirt;
    programs.virt-manager.enable = cfg.libvirt;

    virtualisation.docker.enable = cfg.docker;

    virtualisation.waydroid.enable = cfg.waydroid;

    environment.systemPackages = with pkgs; [
      qemu
    ];

    virtualisation.virtualbox.host.enable = cfg.virtualbox;
    users.extraGroups.vboxusers.members = mkIf cfg.virtualbox [ "ironmoon" ];
  };
}
