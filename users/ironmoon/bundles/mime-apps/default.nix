{
  config,
  lib,
  my-utils,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.mime-apps;
in
{
  options.bundles.mime-apps = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Manage default applications via symlinked mimeapps.list.";
    };
  };

  config = mkIf cfg.enable {
    xdg.configFile."mimeapps.list".source = symlink ./mimeapps.list;
  };
}
