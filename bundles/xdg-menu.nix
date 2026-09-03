{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.xdg-menu;
in
{
  options.bundles.xdg-menu = {
    enable = mkEnableOption "the freedesktop root menu, /etc/xdg/menus/applications.menu, which KF6 ships only inside plasma-workspace; KDE apps outside Plasma register no applications without one";
    source = mkOption {
      type = types.path;
      default = pkgs.writeText "applications.menu" ''
        <!DOCTYPE Menu PUBLIC "-//freedesktop//DTD Menu 1.0//EN"
          "http://www.freedesktop.org/standards/menu-spec/menu-1.0.dtd">
        <Menu>
          <Name>Applications</Name>
          <DefaultAppDirs/>
          <DefaultDirectoryDirs/>
          <DefaultMergeDirs/>
          <Include>
            <All/>
          </Include>
        </Menu>
      '';
      description = "Menu file to install; the default is the spec's example reduced to its defaults, no desktop-specific layout.";
    };
  };

  config = mkIf cfg.enable {
    environment.etc."xdg/menus/applications.menu".source = cfg.source;
  };
}
