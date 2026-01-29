# adapted from home-manager's network-manager-applet modules -- limit to only
# targeting Hyprland
{ pkgs, ... }:
{
  xdg.systemDirs.data = [ "${pkgs.networkmanagerapplet}/share" ];

  systemd.user.services.network-manager-applet = {
    Unit = {
      Description = "Network Manager applet";
      Requires = [ "tray.target" ];
      After = [
        "wayland-session-pre@hyprland.desktop.target"
        "tray.target"
      ];
      # TODO: do we need to restrict this to hyprland?
      PartOf = [ "graphical-session" ];
    };

    Install = {
      WantedBy = [ "graphical-session" ];
    };

    Service = {
      ExecStart = "${pkgs.networkmanagerapplet}/bin/nm-applet --indicator";
    };
  };
}
