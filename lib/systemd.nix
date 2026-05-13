{ lib }:
let
  base = {
    NoNewPrivileges = true;

    PrivateTmp = true;
    PrivateDevices = true;
    ProtectSystem = "strict";
    ProtectHome = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    ProtectClock = true;
    ProtectHostname = true;
    ProtectProc = "invisible";
    ProcSubset = "pid";

    RestrictSUIDSGID = true;
    RestrictRealtime = true;
    RestrictNamespaces = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
      "~@resources"
    ];
  };
in
{
  /**
    Hardening presets
    each is a `serviceConfig` fragment intended to be merged with service-specific settings

    Caveats:
    - `MemoryDenyWriteExecute = true` breaks JIT
    - `ProtectSystem = "strict"` makes the FS read-only; see SANDBOXING docs
    - `SystemCallFilter` blocks `@privileged` and `@resources`
      - breaks services that need setuid, mount, nice/ioprio, etc

    Reference for per-service options see:
    - `man 5 systemd.exec` for common systemd unit config
    - `man 5 systemd.resource-control` for common config for *services*, slices, scopes, sockets, mounts, swaps
    - `man 5 systemd.service` for systemd specific config
    Often relevant:
    - DynamicUser, User, Group
    - RuntimeDirectory, StateDirectory, CacheDirectory, LogsDirectory, ConfigurationDirectory
      - general case: ReadWritePaths
    - CapabilityBoundingSet, AmbientCapabilities
    - IPAddressAllow, IPAddressDeny, RestrictAddressFamilies
    - UMask, KeyringMode, RemoveIPC

    Recall one can run `systemd-analyze security`
  */
  hardening = {
    # Baseline: filesystem + kernel + namespace lockdown.
    inherit base;

    # Network-facing services: baseline + restrict socket families to TCP/UDP/Unix.
    network = base // {
      RestrictAddressFamilies = [
        "AF_INET"
        "AF_INET6"
        "AF_UNIX"
      ];
    };

    # Offline services: baseline + ban network.
    offline = base // {
      PrivateNetwork = true;
      IPAddressDeny = "any";
      RestrictAddressFamilies = [ "AF_UNIX" ];
    };
  };
}
