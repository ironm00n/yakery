{ ... }:
{
  specialisation.hyprland.configuration = {
    system.nixos.tags = [ "hyprland" ];

    bundles.hyprland.enable = true;
    host.hyprland = true;

    # HACK: there was weird behavior when making this common
    # (excluding no specification)
    services.displayManager.sddm.enable = true;
    services.displayManager.sddm.wayland.enable = true;
  };
}
