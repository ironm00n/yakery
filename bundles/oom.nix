{
  config,
  lib,
  ...
}:
let
  inherit (lib)
    genAttrs
    mkEnableOption
    mkOption
    mkIf
    types
    ;
  cfg = config.bundles.oom;
in
{
  options.bundles.oom = {
    enable = mkEnableOption "systemd-oomd memory pressure handling";

    pressureLimit = mkOption {
      type = types.ints.between 1 100;
      default = 50;
      description = "Percentage of wall time the user session may spend fully stalled on memory before oomd kills.";
    };

    pressureDuration = mkOption {
      type = types.str;
      default = "20s";
      description = "How long `pressureLimit` must be exceeded before oomd acts.";
    };

    killOnSwapExhaustion = mkOption {
      type = types.bool;
      default = true;
      description = "Also let oomd kill from the root slice once swap is nearly full.";
    };

    lastResortUserUnits = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "wayland-wm@" ];
      description = "User units oomd may only kill when no other candidate is left.";
    };

    enableSysrq = mkOption {
      type = types.bool;
      default = true;
      description = "Full SysRq, for escaping a livelock without the power button.";
    };
  };

  config = mkIf cfg.enable {
    systemd.oomd.settings.OOM.DefaultMemoryPressureDurationSec = cfg.pressureDuration;

    # systemd-oomd(8) wants swap monitoring on the root slice and pressure
    # monitoring strictly below it, where a kill can pick a single app.
    systemd.slices = mkIf cfg.killOnSwapExhaustion {
      "-" = {
        overrideStrategy = "asDropin";
        sliceConfig.ManagedOOMSwap = "kill";
      };
    };

    # Pressure monitoring belongs on the system-level user@.service, the unit
    # systemd-oomd(8) names under USAGE RECOMMENDATIONS: PID 1 is what
    # advertises managed cgroups to oomd, so the equivalent drop-in inside the
    # user manager is never reported. This is also why systemd.oomd.enableUserSlices
    # is unused — besides emitting its drop-in as "slice" rather than "-.slice",
    # it writes to the user manager, and monitoring user.slice instead would make
    # oomd ignore ManagedOOMPreference on everything below user@$UID.service.
    systemd.services."user@" = {
      overrideStrategy = "asDropin";
      serviceConfig = {
        ManagedOOMMemoryPressure = "kill";
        ManagedOOMMemoryPressureLimit = "${toString cfg.pressureLimit}%";
      };
    };

    systemd.user.services = genAttrs cfg.lastResortUserUnits (_: {
      overrideStrategy = "asDropin";
      serviceConfig.ManagedOOMPreference = "avoid";
    });

    # The default of 16 is sync-only, which leaves the power button as the only
    # exit from a livelock. 1 also enables SysRq+F and the W/M diagnostic dumps.
    boot.kernel.sysctl = mkIf cfg.enableSysrq { "kernel.sysrq" = 1; };
  };
}
