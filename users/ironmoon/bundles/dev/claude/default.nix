{
  config,
  lib,
  my-utils,
  pkgs-master,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mapAttrs'
    nameValuePair
    filterAttrs
    ;
  inherit (my-utils) symlink;
  inherit (pkgs-master) claude-code;
  cfg = config.bundles.dev.claude;

  # claude code loads any ~/.claude/skills/<name> holding a .claude-plugin/ as a plugin
  mods = mapAttrs' (name: _: nameValuePair ".claude/skills/${name}" { source = symlink (./mods + "/${name}"); }) (
    filterAttrs (_: type: type == "directory") (builtins.readDir ./mods)
  );
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
      ".claude/hooks/".source = symlink ./hooks;
      ".claude/my-marketplaces".source = symlink ./marketplaces;
    }
    // mods;
  };
}
