{
  config,
  lib,
  my-utils,
  pkgs-master,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  inherit (pkgs-master) claude-code;
  cfg = config.bundles.dev.claude;
in
{
  options.bundles.dev.claude = {
    enable = mkEnableOption "claude code";
  };

  config = mkIf cfg.enable {
    home.packages = [ claude-code ];

    home.file = {
      # no XDG: https://github.com/anthropics/claude-code/issues/1455
      ".claude/CLAUDE.md".source = symlink ./CLAUDE.md;
      ".claude/settings.json".source = symlink ./settings.json;
    };
  };
}
