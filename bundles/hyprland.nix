# also see: ./hosts/common/specializations/hyprland.nix
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.hyprland;
in
{
  options.bundles.hyprland = {
    enable = mkEnableOption "hyprland related config";
  };

  config = mkIf cfg.enable {
    security.polkit.enable = true;
    security.polkit.debug = true;

    programs.hyprland = {
      enable = true;
      withUWSM = true;
    };
    programs.hyprlock.enable = true;
    services.hypridle.enable = true;

    qt.enable = true;

    xdg.portal = {
      enable = true;
      xdgOpenUsePortal = true;
      configPackages =
        with pkgs;
        lib.mkForce [
          hyprland
        ];
      extraPortals =
        with pkgs;
        lib.mkForce [
          xdg-desktop-portal-gtk
          # patched xdph for the Zoom screenshare fix (see packages/xdg-desktop-portal-hyprland-zoom)
          (callPackage ../packages/xdg-desktop-portal-hyprland-zoom { })
          kdePackages.xdg-desktop-portal-kde # for dolphin
        ];
    };

    security.pam.services.login.enableGnomeKeyring = true;
    security.pam.services.sddm.enableGnomeKeyring = true;
    services.gnome.gnome-keyring.enable = true;

    environment.systemPackages = with pkgs; [
      hyprland
      hyprpicker
      wofi
      cliphist
      brightnessctl
      hyprpicker
      hypridle
      hyprlock
      hyprshot
      hyprpolkitagent

      kdePackages.breeze
      sddm-astronaut
    ];
    services.displayManager.sddm = {
      package = pkgs.kdePackages.sddm;
      theme = "sddm-astronaut-theme";
      extraPackages = with pkgs; [
        sddm-astronaut
        kdePackages.qtmultimedia
        kdePackages.qtsvg
        kdePackages.qtvirtualkeyboard
      ];
    };
  };
}
