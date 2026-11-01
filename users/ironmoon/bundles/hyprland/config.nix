{ pkgs }:
{
  enable = true;

  configType = "lua";

  # conflicts with uwsm
  systemd.enable = false;

  plugins = with pkgs; [
    # FIXME: crashes
    # hyprlandPlugins.hyprexpo
    # hyprlandPlugins.hyprspace
  ];
}
