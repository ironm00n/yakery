{
  config,
  lib,
  pkgs,
  ...
}:
{
  systemd.user.services.anyrun-daemon = lib.mkIf config.host.hyprland {
    Unit = {
      Description = "anyrun launcher daemon";
      PartOf = [ "graphical-session.target" ];
      After = [ "graphical-session.target" ];
    };

    Install = {
      WantedBy = [ "graphical-session.target" ];
    };

    Service = {
      ExecStart = "${lib.getExe pkgs.anyrun} daemon";
      Restart = "on-failure";
    };
  };
}
