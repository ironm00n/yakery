# One Microsoft 365 calendar mirrored into one Stalwart calendar. nginx hands the sync Graph's
# notifications and the sign-in callback on a loopback socket; the sync holds the app's private
# key and the sign-in's refresh token in its state directory, and nothing else does.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.outlook-sync;
  inherit (lib) mkOption types;
  user = "outlook-sync";
  state = "/var/lib/${user}";
  policy = {
    inherit (cfg)
      clientId
      tenant
      public
      stalwart
      account
      calendar
      owner
      daysBack
      daysAhead
      write
      ;
  };
  # every command runs as the service, on its state
  command = pkgs.writeShellScriptBin "outlook-sync" ''
    exec ${lib.getExe' pkgs.util-linux "runuser"} -u ${user} -- \
      env STATE_DIRECTORY=${state} ${lib.getExe cfg.package} "$@"
  '';
in
{
  options.services.outlook-sync = {
    enable = lib.mkEnableOption "a Microsoft 365 calendar mirrored into a Stalwart calendar";
    clientId = mkOption {
      type = types.str;
      description = "The Entra app's client id, whose certificate credential the sync's key is.";
    };
    tenant = mkOption {
      type = types.str;
      default = "organizations";
    };
    public = mkOption {
      type = types.strMatching "https://[^/]+/.+[^/]";
      description = "The HTTPS prefix nginx forwards to the sync, without a trailing slash; `/callback` under it is the app's redirect URI.";
    };
    stalwart = mkOption {
      type = types.str;
      default = "127.0.0.1:8080";
    };
    account = mkOption {
      type = types.str;
      description = "The Stalwart account the sync writes as, which the calendar is shared with to write.";
    };
    appPassword = mkOption {
      type = types.str;
      description = "File holding the account's app password.";
    };
    calendar = mkOption {
      type = types.str;
      description = "The mirror calendar's CalDAV path on Stalwart.";
    };
    owner = mkOption {
      type = types.str;
      description = "The address of the Stalwart account that holds the mirror.";
    };
    write = lib.mkEnableOption ''
      sending the owner's edits in the mirror back to Outlook. The sign-in then asks for
      Calendars.ReadWrite, which the tenant's admin may have to grant, and the sync listens to
      Stalwart's push for changes in the owner's account
    '';
    daysBack = mkOption {
      type = types.ints.positive;
      default = 7;
    };
    daysAhead = mkOption {
      type = types.ints.positive;
      default = 63;
      description = "How far ahead the mirror reaches: past the scheduler's longest link window and its horizon.";
    };
    port = mkOption {
      type = types.port;
      default = 8932;
    };
    package = mkOption {
      type = types.package;
      default = pkgs.callPackage ../crates/outlook-sync/package.nix { inherit policy; };
      defaultText = lib.literalExpression "the sync built with its policy";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ command ];
    users.groups.${user} = { };
    users.users.${user} = {
      isSystemUser = true;
      group = user;
    };

    systemd.sockets.outlook-sync = {
      wantedBy = [ "sockets.target" ];
      listenStreams = [ "127.0.0.1:${toString cfg.port}" ];
      socketConfig.FileDescriptorName = "http";
    };

    systemd.services.outlook-sync-key = {
      description = "The sync's private key and self-signed certificate, made once";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = user;
        StateDirectory = user;
        UMask = "0077";
      };
      script = ''
        if [ ! -s ${state}/key.pem ]; then
          ${lib.getExe pkgs.openssl} req -x509 -newkey rsa:3072 -nodes -days 3650 -subj /CN=outlook-sync \
            -keyout ${state}/key.pem.new -out ${state}/certificate.pem
          mv ${state}/key.pem.new ${state}/key.pem
        fi
      '';
    };

    systemd.services.outlook-sync = {
      requires = [
        "outlook-sync.socket"
        "outlook-sync-key.service"
      ];
      after = [
        "outlook-sync-key.service"
        "stalwart.service"
        "stalwart-apply-plan.service"
      ];
      wantedBy = [ "multi-user.target" ];
      unitConfig.StartLimitIntervalSec = 0;
      serviceConfig = {
        User = user;
        Group = user;
        ExecStart = "${lib.getExe cfg.package} serve";
        StateDirectory = user;
        LoadCredential = "app-password:${cfg.appPassword}";
        Restart = "on-failure";
        RestartSec = 5;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictNamespaces = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
        ];
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
        UMask = "0077";
        # the time zones the owner's calendar app names
        Environment = "TZDIR=${pkgs.tzdata}/share/zoneinfo";
      };
    };
  };
}
