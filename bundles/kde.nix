# also see: ./hosts/common/specializations/kde.nix
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.kde;
in
{
  options.bundles.kde = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable KDE related config.";
    };
  };

  config = mkIf cfg.enable {
    services.xserver.enable = true;
    services.desktopManager.plasma6.enable = true;

    # TODO: some of these may be already set in the plasma6 module
    xdg.portal = {
      enable = true;
      xdgOpenUsePortal = true;
      configPackages =
        with pkgs;
        lib.mkForce [
          kdePackages.plasma-workspace
        ];
      extraPortals =
        with pkgs;
        lib.mkForce [
          kdePackages.kwallet
          kdePackages.xdg-desktop-portal-kde
        ];
    };

    environment.systemPackages = with pkgs; [
      maliit-keyboard
    ];
  };
}
