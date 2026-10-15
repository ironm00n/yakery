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
    # a copy, not a link: Claude Code watches the directory a settings file resolves into, and once
    # native watching falls back to polling, a link into /nix/store has it stat every store entry
    environment.etc."claude-code/managed-settings.json" = {
      source = format.generate "managed-settings.json" managedSettings;
      mode = "0444";
    };
  };
}
