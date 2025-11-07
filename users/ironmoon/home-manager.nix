args@{
  lib,
  pkgs,
  host,
  my-utils,
  ...
}:
let
  _ = my-utils;
  my-modules = import ./modules/default.nix;
  bundles = import ./bundles/default.nix;
  importWith = path: import path args;
  fw12 = "framework-12-13th-gen-intel";
in
{
  imports = [
    ../../hosts/options.nix
    ./services/network-manager-applet.nix
    ./services/kbuildsycoca6.nix
    ./conf/theme.nix
    ./conf/xdg.nix
  ]
  ++ my-modules
  ++ bundles;

  host = host;

  bundles = {
    eww.enable = false;
    waybar.enable = true;
    quickshell.enable = false;
    zsh.enable = true;
    dev.enable = true;
    dev.jetbrains = !host.lightweight;
    sec.enable = !host.lightweight;
    emacs.enable = true;
    nvim.enable = host.id == fw12;

    discord.enable = true;
  };

  home.sessionVariables = {
    PAGER = "${lib.getExe pkgs.moor} --no-linenumbers";
    EDITOR = "${lib.getExe pkgs.neovim}";
    VISUAL = "${lib.getExe pkgs.neovim}";
    ELECTRON_OZONE_PLATFORM_HINT = "auto";
  };

  programs = {
    fzf = importWith ./programs/fzf.nix;
    konsole = importWith ./programs/konsole.nix;
    okular = importWith ./programs/okular.nix;
    git = importWith ./programs/git.nix;
    firefox = importWith ./programs/firefox.nix;
    direnv = importWith ./programs/direnv.nix;

    hyprlock = importWith ./programs/hyprlock.nix;
    kitty = importWith ./programs/kitty.nix;
    anyrun = importWith ./programs/anyrun.nix;
  };

  services = {
    dunst = importWith ./services/dunst.nix;
    hypridle = importWith ./services/hypridle.nix;
    hyprpaper = importWith ./services/hyprpaper.nix;
    hyprpolkitagent = importWith ./services/hyprpolkitagent.nix;
    syncthing = importWith ./services/syncthing.nix;
  };

  programs.plasma = importWith ./env/plasma.nix;
  wayland.windowManager.hyprland = importWith ./env/hyprland.nix;

  # The state version is required and should stay at the version you
  # originally installed.
  home.stateVersion = "24.05";
}
