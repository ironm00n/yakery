# sudo -E gpclient --fix-openssl connect -portal vpn.northeastern.edu --default-browser
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.vpn.globalprotect;
in
{
  options.bundles.vpn.globalprotect.enable = mkEnableOption "GlobalProtect VPN client (gpclient)";

  config = mkIf cfg.enable {
    environment.systemPackages = with pkgs; [
      gpauth
      gpclient
      # Handler for the globalprotectcallback: scheme; lets external-browser SSO
      # (`gpclient connect --browser`) hand the auth cookie back to gpauth.
      (makeDesktopItem {
        name = "gpclient-callback";
        desktopName = "GlobalProtect Callback";
        exec = "${gpclient}/bin/gpclient launch-gui %u";
        mimeTypes = [ "x-scheme-handler/globalprotectcallback" ];
        noDisplay = true;
      })
    ];

    xdg.mime.defaultApplications."x-scheme-handler/globalprotectcallback" = "gpclient-callback.desktop";
  };
}
