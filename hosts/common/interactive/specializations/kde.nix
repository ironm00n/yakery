{ ... }:
{
  specialisation.kde.configuration = {
    system.nixos.tags = [ "kde" ];

    bundles.kde.enable = true;
    host.kde = true;

    services.displayManager.sddm.enable = true;
    services.displayManager.sddm.wayland.enable = true;
    services.displayManager.sddm.wayland.compositor = "kwin";
  };
}
