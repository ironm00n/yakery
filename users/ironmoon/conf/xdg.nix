{ pkgs, ... }:
{
  xdg.enable = true;
  xdg.userDirs.enable = true;

  home.preferXdgDirectories = true;

  # TODO: work on making these specialized for each DE
  # hyprland enables home-manager xdg config, while plasma doesn't. So we need to
  # set all these here.
  xdg.portal = {
    xdgOpenUsePortal = true;
    configPackages =
      with pkgs;
      lib.mkForce [
        kdePackages.plasma-workspace
        hyprland
      ];
    extraPortals =
      with pkgs;
      lib.mkForce [
        kdePackages.kwallet
        kdePackages.xdg-desktop-portal-kde
        xdg-desktop-portal-gtk
        xdg-desktop-portal-hyprland
      ];
  };

}
