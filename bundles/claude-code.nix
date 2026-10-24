{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.claude-code;
  format = pkgs.formats.json { };
  managedSettings = {
    # a Team/Enterprise plan seats cc-plugin-sec-default, which skips user mods on prompt.* and ignores user appendPlugins; a managed list without it unseats it
    prependPlugins = [ ];
    cleanupPeriodDays = 1000000;
  };
in
{
  options.bundles.claude-code.enable = mkEnableOption "Claude Code's machine-wide managed settings, /etc/claude-code/managed-settings.json, which outrank every user, project and remote source";

  config = mkIf cfg.enable {
    environment.etc."claude-code/managed-settings.json".source =
      format.generate "managed-settings.json" managedSettings;
  };
}
