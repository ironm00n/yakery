# agentd's socket units and service, for the VM and for nix/tests/agentd.nix alike.
{
  key,
  keyUnit ? null,
  quotaDevice,
  sessions,
  apiAddress,
  claude,
}:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  agentd = pkgs.callPackage ../../../crates/rust.nix { crate = "agentd"; };
  runtimeDirectory = "agentd";
  runDir = "/run/${runtimeDirectory}";
  # agentd takes each by its name, and checks the control port against agent-proto's CONTROL_PORT
  sockets = {
    api = {
      ListenStream = "${runDir}/api.sock";
      SocketMode = "0666";
    };
    direct.ListenStream = "${apiAddress}:443";
    control.ListenStream = "vsock::6000";
  };
  units = map (name: "agentd-${name}.socket") (lib.attrNames sockets);
  keyUnits = lib.optional (keyUnit != null) keyUnit;
in
{
  systemd.sockets = lib.mapAttrs' (
    name: socket:
    lib.nameValuePair "agentd-${name}" {
      wantedBy = [ "sockets.target" ];
      socketConfig = socket // {
        FileDescriptorName = name;
        Service = "agentd.service";
      };
    }
  ) sockets;

  systemd.services.agentd = {
    description = "Agent sessions and their way to the gateway";
    wantedBy = [ "multi-user.target" ];
    requires = keyUnits ++ units;
    after = keyUnits ++ units;
    startLimitIntervalSec = 0;
    path = [ config.systemd.package ];
    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe' agentd "agentd")
        "--agents"
        (import ./agents-json.nix { inherit lib pkgs; })
        "--key"
        key
        "--claude"
        claude
        "--mkdir"
        (lib.getExe' pkgs.coreutils "mkdir")
        "--quota-device"
        quotaDevice
        "--sessions"
        (toString sessions)
        "--run-dir"
        runDir
      ];
      Sockets = units;
      RuntimeDirectory = runtimeDirectory;
      RuntimeDirectoryMode = "0755";
      # the api socket lives here, and its socket unit keeps listening while agentd restarts
      RuntimeDirectoryPreserve = "yes";
      OOMScoreAdjust = -1000;
      Restart = "always";
      RestartSec = "1s";
    };
  };
}
