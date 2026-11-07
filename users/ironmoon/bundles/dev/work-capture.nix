# Files every request Owen types to an agent session as an agents/work issue (notes repo tools/work-capture). One host
# runs it: each keeps its own seen file, so a second would file everything twice.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (config.home) homeDirectory username;
in
{
  options.bundles.dev.work-capture = lib.mkEnableOption "the work-capture pass every 15 minutes";

  config = lib.mkIf (config.bundles.dev.enable && config.bundles.dev.work-capture) {
    systemd.user.services.work-capture = {
      Unit.Description = "File Owen's typed requests as agents/work issues";
      Service = {
        Type = "oneshot";
        ExecStart = "${lib.getExe pkgs.bash} ${homeDirectory}/workspace/claude/tools/work-capture/run.sh";
        # claude-tap from his `nix profile`, and his python, as his sessions find them
        Environment = [
          "PATH=${homeDirectory}/.nix-profile/bin:/etc/profiles/per-user/${username}/bin:/run/current-system/sw/bin"
        ];
      };
    };
    systemd.user.timers.work-capture = {
      Unit.Description = "work-capture every 15 minutes";
      Timer = {
        OnCalendar = "*:0/15";
        Persistent = true;
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
