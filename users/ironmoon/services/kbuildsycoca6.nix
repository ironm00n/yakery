{
  pkgs,
  config,
  lib,
  ...
}:
{
  systemd.user.services.kbuildsycoca6 = lib.mkIf config.host.hyprland {
    Unit = {
      Description = "Regenerate KDE service cache";
      After = [ "graphical-session.target" ];
    };

    Install = {
      WantedBy = [ "default.target" ];
    };

    Service = {
      Type = "oneshot";
      ExecStart = "${pkgs.kdePackages.kservice}/bin/kbuildsycoca6";
    };
  };

  systemd.user.paths.kbuildsycoca6 = lib.mkIf config.host.hyprland {
    Unit = {
      Description = "Watch for application changes";
    };
    Install = {
      WantedBy = [ "default.target" ];
    };
    Path = {
      PathChanged = [
        "/run/current-system"
        "%h/.nix-profile"
      ];
    };
  };
}
