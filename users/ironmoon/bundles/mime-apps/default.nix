{
  config,
  lib,
  my-utils,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.mime-apps;
in
{
  options.bundles.mime-apps = {
    enable = mkEnableOption "manage default applications via symlinked mimeapps.list";
  };

  config = mkIf cfg.enable {
    xdg.configFile."mimeapps.list".source = symlink ./mimeapps.list;
  };
}
