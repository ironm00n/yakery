{ pkgs, ... }:
let
  kwriteconfig = "${pkgs.kdePackages.kconfig}/bin/kwriteconfig6";
  # TODO: ksystemlog, kalarm
  rcFiles = [
    "dolphinrc"
    "kwriterc"
    "katerc"
    "konsolerc"
    "arkrc"
    "okularrc"
    "kcalcrc"
    "ktimerrc"
    "kalarmrc"
  ];
in
{
  systemd.user.services.fix-kde-color-schemes = {
    Unit = {
      Description = "Force ColorScheme in KDE apps.";
      After = [
        "wayland-session@hyprland.desktop.target"
      ];
    };

    Service = {
      Type = "oneshot";
      ExecStart = pkgs.writeShellScript "fix-kde-color-schemes" ''
        set -eu
        for f in ${builtins.concatStringsSep " " rcFiles}; do
          ${kwriteconfig} --file "$f" --group UiSettings --key ColorScheme '*'
        done
      '';
    };

    Install = {
      WantedBy = [
        "wayland-session@hyprland.desktop.target"
      ];
    };
  };
}
