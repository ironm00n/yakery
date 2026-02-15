{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.nvidia;
in
{
  options.bundles.nvidia = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable NVIDIA specific options.";
    };
  };

  config = mkIf cfg.enable {
    # Enable OpenGL
    hardware.graphics = {
      enable = true;
    };
    hardware.nvidia = {
      modesetting.enable = true;
      powerManagement.enable = false;
      powerManagement.finegrained = false;
      # these are not yet ready:
      open = true;
      nvidiaSettings = true; # nvidia-settings

      # TODO: remove once https://github.com/NixOS/nixpkgs/issues/489947 is fixed
      package =
        let
          base = config.boot.kernelPackages.nvidiaPackages.latest;
          cachyos-nvidia-patch = pkgs.fetchpatch {
            url = "https://raw.githubusercontent.com/CachyOS/CachyOS-PKGBUILDS/master/nvidia/nvidia-utils/kernel-6.19.patch";
            sha256 = "sha256-YuJjSUXE6jYSuZySYGnWSNG5sfVei7vvxDcHx3K+IN4=";
          };
          driverAttr = if config.hardware.nvidia.open then "open" else "bin";
        in
        base
        // {
          ${driverAttr} = base.${driverAttr}.overrideAttrs (oldAttrs: {
            patches = (oldAttrs.patches or [ ]) ++ [ cachyos-nvidia-patch ];
          });
        };
    };

    # Load nvidia driver for Xorg and Wayland
    services.xserver.videoDrivers = [ "nvidia" ];
  };
}
