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
  fw13 = (import ../../hosts/fw13/host-cfg.nix { inherit pkgs; }).id;
  fw12 = (import ../../hosts/fw12/host-cfg.nix { inherit pkgs; }).id;
  desktop = (import ../../hosts/desktop/host-cfg.nix { inherit pkgs; }).id;
in
{
  imports = [
    ../../hosts/options.nix
    ./services/network-manager-applet.nix
    ./services/kbuildsycoca6.nix
    ./services/fix-kde-colorscheme.nix
    ./services/anyrun-daemon.nix
    ./conf/xdg.nix
  ]
  ++ my-modules
  ++ bundles;

  host = host;

  bundles = {
    hyprland.enable = host.hyprland;
    theme.enable = host.hyprland;

    kitty.enable = true;

    eww.enable = false;
    waybar.enable = true;
    quickshell.enable = false;
    zsh.enable = true;
    mime-apps.enable = true;
    dev.enable = true;
    dev.jetbrains = !host.lightweight;
    dev.langs = host.id != fw12;
    sec.enable = !host.lightweight;
    emacs.enable = true;
    nvim.enable = host.id != fw13;

    syncthing.enable = true;
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
    lf = importWith ./programs/lf.nix;
    konsole = importWith ./programs/konsole.nix;
    okular = importWith ./programs/okular.nix;
    git = importWith ./programs/git.nix;
    firefox = importWith ./programs/firefox.nix;
    direnv = importWith ./programs/direnv.nix;

    hyprlock = importWith ./programs/hyprlock.nix;
    anyrun = importWith ./programs/anyrun.nix;
  };

  services = {
    dunst = importWith ./services/dunst.nix;
    hypridle = importWith ./services/hypridle.nix;
    hyprpaper = importWith ./services/hyprpaper.nix;
    hyprpolkitagent = importWith ./services/hyprpolkitagent.nix;
  };

  programs.plasma = importWith ./env/plasma.nix;

  programs.zellij.enable = true;

  # The state version is required and should stay at the version you
  # originally installed.
  home.stateVersion = "24.05";
}
