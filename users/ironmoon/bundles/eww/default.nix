{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.eww;
  ewwDir = "${config.xdg.configHome}/eww";
in
{
  options.bundles.eww = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable eww bar.";
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ pkgs.eww ];

    # home.file."${ewwDir}/eww.yuck".source = symlink ./eww.yuck;
    # home.file."${ewwDir}/eww.scss".source = symlink ./eww.scss;
    home.file."${ewwDir}/eww.yuck".source = symlink "ironmoon/bundles/eww/eww.yuck";
    home.file."${ewwDir}/eww.scss".source = symlink "ironmoon/bundles/eww/eww.scss";
  };
}
