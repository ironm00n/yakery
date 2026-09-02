{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf mkMerge;
  cfg = config.bundles.appboxes;
  driverFile = rel: containerPath: {
    hostPath = "/run/opengl-driver/${rel}";
    inherit containerPath;
  };
in
{
  options.bundles.appboxes = {
    enable = mkEnableOption "mutable per-app distro containers (rootless podman + distrobox)";
  };

  config = mkIf cfg.enable (mkMerge [
    {
      virtualisation.podman.enable = true;
      environment.systemPackages = [ pkgs.distrobox ];
      environment.etc."distrobox/distrobox.conf".text = ''
        container_manager="podman"
      '';
    }
    (mkIf config.host.nvidia {
      # NixOS keeps the driver's loader JSONs under /run/opengl-driver/share while
      # FHS loaders search /usr/share and /etc. egl_vendor.d goes file by file so
      # nix mesa's 50_mesa.json can't shadow the box's own mesa.
      hardware.nvidia-container-toolkit = {
        enable = true;
        mounts = [
          (driverFile "share/glvnd/egl_vendor.d/10_nvidia.json" "/usr/share/glvnd/egl_vendor.d/10_nvidia.json")
          (driverFile "share/egl/egl_external_platform.d" "/usr/share/egl/egl_external_platform.d")
          (driverFile "share/vulkan/icd.d/nvidia_icd.json" "/etc/vulkan/icd.d/nvidia_icd.json")
          (driverFile "share/vulkan/implicit_layer.d/nvidia_layers.json" "/etc/vulkan/implicit_layer.d/nvidia_layers.json")
        ];
      };
    })
  ]);
}
