{
  config,
  lib,
  pkgs,
  utils,
  my-lib,
  ...
}:
let
  inherit (lib)
    concatMap
    genAttrs
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.borg-hop;

  stateDir = "/var/lib/borg-hop";
  identity = "${stateDir}/id_ed25519";

  serve = [
    cfg.target.remotePath
    "serve"
    "--append-only"
  ]
  ++ concatMap (repo: [
    "--restrict-to-repository"
    repo
  ]) cfg.target.repositories;
in
{
  options.bundles.borg-hop = {
    enable = mkEnableOption "TCP relay into a remote `borg serve`, so the SSH hop is terminated next to the repository";

    port = mkOption {
      type = types.port;
      default = 8023;
      description = "Port the borg RPC stream is accepted on.";
    };

    interfaces = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "nb-netbird" ];
      description = "Interfaces the port is opened on. The relay authenticates nothing itself; keep this to trusted overlays.";
    };

    target = my-lib.borg.targetOptions // {
      repositories = mkOption {
        type = types.listOf types.str;
        description = "Absolute repository paths on the target the relay may open.";
      };
    };
  };

  config = mkIf cfg.enable {
    users.users.borg-hop = {
      isSystemUser = true;
      group = "borg-hop";
      home = stateDir;
    };
    users.groups.borg-hop = { };

    programs.ssh.knownHosts.borg-hop-target = {
      hostNames = [ (my-lib.borg.knownHostName cfg.target) ];
      publicKey = cfg.target.hostPublicKey;
    };

    systemd.services.borg-hop-keygen = {
      description = "borg-hop client identity";
      wantedBy = [ "multi-user.target" ];
      unitConfig.ConditionPathExists = "!${identity}";
      serviceConfig = {
        Type = "oneshot";
        User = "borg-hop";
        StateDirectory = "borg-hop";
        ExecStart = utils.escapeSystemdExecArgs [
          "${pkgs.openssh}/bin/ssh-keygen"
          "-q"
          "-t"
          "ed25519"
          "-N"
          ""
          "-C"
          "borg-hop@${config.networking.hostName}"
          "-f"
          identity
        ];
      };
    };

    systemd.sockets.borg-hop = {
      description = "borg-hop relay";
      wantedBy = [ "sockets.target" ];
      listenStreams = [ (toString cfg.port) ];
      socketConfig.Accept = true;
    };

    systemd.services."borg-hop@" = {
      description = "borg-hop relay connection";
      requires = [ "borg-hop-keygen.service" ];
      after = [ "borg-hop-keygen.service" ];
      serviceConfig = {
        User = "borg-hop";
        StateDirectory = "borg-hop";
        StandardInput = "socket";
        StandardOutput = "socket";
        StandardError = "journal";
        ExecStart = utils.escapeSystemdExecArgs (
          [
            "${pkgs.openssh}/bin/ssh"
            "-o"
            "BatchMode=yes"
            "-o"
            "StrictHostKeyChecking=yes"
            "-o"
            "UserKnownHostsFile=/dev/null"
            "-i"
            identity
            "-p"
            (toString cfg.target.port)
            "${cfg.target.user}@${cfg.target.host}"
          ]
          ++ serve
        );
      };
    };

    networking.firewall.interfaces = genAttrs cfg.interfaces (_: {
      allowedTCPPorts = [ cfg.port ];
    });

    # The relay terminates a long-fat-pipe TCP stream in both directions: the
    # stock 4-6 MiB buffer caps would bound it at ~40-60 MiB/s per 100 ms of
    # RTT, and cubic collapses on the WireGuard leg's sub-percent loss.
    boot.kernelModules = [ "tcp_bbr" ];
    boot.kernel.sysctl = {
      "net.ipv4.tcp_congestion_control" = "bbr";
      "net.ipv4.tcp_rmem" = "4096 131072 33554432";
      "net.ipv4.tcp_wmem" = "4096 16384 33554432";
    };
  };
}
