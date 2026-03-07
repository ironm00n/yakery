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
      After = [ "wayland-session@hyprland.desktop.target" ];
    };

    Install = {
      WantedBy = [ "wayland-session@hyprland.desktop.target" ];
    };

    Service = {
      Type = "oneshot";
      # UWSM sets XDG_MENU_PREFIX=hyprland- by default, but kbuildsycoca6
      # needs plasma- to find the KDE application menu database.
      Environment = "XDG_MENU_PREFIX=plasma-";
      ExecStart = "${pkgs.kdePackages.kservice}/bin/kbuildsycoca6";
    };
  };

  systemd.user.paths.kbuildsycoca6 = lib.mkIf config.host.hyprland {
    Unit = {
      Description = "Watch for application changes";
    };
    Install = {
      WantedBy = [ "wayland-session@hyprland.desktop.target" ];
    };
    Path = {
      PathChanged = [
        "/run/current-system"
        "%h/.nix-profile"
      ];
    };
  };
}
