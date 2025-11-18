# also see: ./hosts/common/specializations/hyprland.nix
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.hyprland;
in
{
  options.bundles.hyprland = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable hyprland related config.";
    };
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
          xdg-desktop-portal-hyprland
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
