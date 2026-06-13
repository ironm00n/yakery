{ config, lib, pkgs, my-utils,  ... }:

let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.theme;
in
{
  options.bundles.theme = {
    enable = mkEnableOption "security stuff";
  };

  config = mkIf cfg.enable {
    home.pointerCursor = {
      enable = true;
      gtk.enable = true;
      name = "breeze_cursors";
      package = pkgs.kdePackages.breeze;
      size = 24;
    };

    qt = {
      enable = true;
      platformTheme.name = "qt6ct";
    };

    gtk = {
      enable = true;
      theme = {
        name = "Breeze-Dark";
        package = pkgs.kdePackages.breeze-gtk;
      };

      iconTheme = {
        package = pkgs.kdePackages.breeze-icons;
        name = "breeze-dark";
      };
      gtk2.force = true;
    };

    gtk.gtk4.theme = config.gtk.theme;

    dconf = {
      enable = true;
      settings = {
        "org/gnome/desktop/interface" = {
          color-scheme = "prefer-dark";
        };
      };
    };

    xdg.configFile."qt6ct/qt6ct.conf".source = symlink ./qt6ct.conf;
    xdg.configFile."qt6ct/style-colors.conf".source = symlink ./style-colors.conf;
  };
}
