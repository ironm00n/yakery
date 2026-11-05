# The scheduler's two services and three sockets: the front serves nginx on a socket it is
# handed and can open no IP socket of its own; the core holds the app password and the links.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.scheduler;
  inherit (lib) mkOption types;
  layout = {
    http.fd = "http";
    front = {
      fd = "front";
      path = "/run/scheduler/front.sock";
    };
    admin = {
      fd = "admin";
      path = "/run/scheduler/admin.sock";
    };
  };
  policy = {
    inherit layout;
    account = {
      address = cfg.account;
      inherit ((import ../crates/scheduler/accounts.nix).core) permissions;
    };
    inherit (cfg)
      groups
      host
      stalwart
      zone
      hours
      ;
    notice = "PT${toString cfg.noticeHours}H";
    horizonWeeks = cfg.horizonWeeks;
    gap = "PT${toString cfg.gapMinutes}M";
  };
  hardened = {
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
    UMask = "0077";
    Environment = "TZDIR=${pkgs.tzdata}/share/zoneinfo";
  };
in
{
  options.services.scheduler = {
    enable = lib.mkEnableOption "schedule pages and one-time booking links on Stalwart";
    account = mkOption {
      type = types.str;
      description = "The address the core signs in as; whoever shares a Bookings calendar with it is whose busy time is shown and who is booked.";
    };
    appPassword = mkOption {
      type = types.str;
      description = "File holding the account's app password.";
    };
    groups = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "Addresses of groups whose calendars shared with the account count too.";
    };
    host = mkOption { type = types.str; };
    stalwart = mkOption {
      type = types.str;
      default = "127.0.0.1:8080";
    };
    zone = mkOption {
      type = types.str;
      default = "America/New_York";
    };
    hours = mkOption {
      type = types.listOf types.str;
      description = "Bookable hours, each like `mon 10:00-18:00`; the package's build refuses one that doesn't parse.";
      default = map (day: "${day} 10:00-18:00") [
        "mon"
        "tue"
        "wed"
        "thu"
        "fri"
      ];
    };
    noticeHours = mkOption {
      type = types.ints.unsigned;
      default = 4;
    };
    horizonWeeks = mkOption {
      type = types.ints.between 1 8;
      default = 4;
    };
    gapMinutes = mkOption {
      type = types.ints.between 5 1440;
      default = 5;
    };
    port = mkOption {
      type = types.port;
      default = 8931;
    };
    policy = mkOption {
      readOnly = true;
      default = policy;
      description = "What the package compiles in.";
    };
    package = mkOption {
      type = types.package;
      default = pkgs.callPackage ../crates/scheduler/package.nix { inherit policy; };
      defaultText = lib.literalExpression "the scheduler built with `policy`";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    users.groups.scheduler-front = { };
    users.users = lib.genAttrs [ "scheduler-front" "scheduler-core" ] (name: {
      isSystemUser = true;
      group = if name == "scheduler-front" then "scheduler-front" else "nogroup";
    });

    systemd.sockets = {
      scheduler-http = {
        wantedBy = [ "sockets.target" ];
        listenStreams = [ "127.0.0.1:${toString cfg.port}" ];
        socketConfig = {
          FileDescriptorName = layout.http.fd;
          Service = "scheduler-front.service";
        };
      };
      scheduler-front = {
        wantedBy = [ "sockets.target" ];
        listenStreams = [ layout.front.path ];
        socketConfig = {
          FileDescriptorName = layout.front.fd;
          Service = "scheduler-core.service";
          SocketUser = "scheduler-core";
          SocketGroup = "scheduler-front";
          SocketMode = "0660";
        };
      };
      scheduler-admin = {
        wantedBy = [ "sockets.target" ];
        listenStreams = [ layout.admin.path ];
        socketConfig = {
          FileDescriptorName = layout.admin.fd;
          Service = "scheduler-core.service";
          SocketMode = "0600";
        };
      };
    };

    systemd.services.scheduler-front = {
      requires = [ "scheduler-http.socket" ];
      serviceConfig = hardened // {
        User = "scheduler-front";
        Group = "scheduler-front";
        Sockets = [ "scheduler-http.socket" ];
        ExecStart = "${lib.getExe cfg.package} front";
        RestrictAddressFamilies = "AF_UNIX";
        IPAddressDeny = "any";
      };
    };

    systemd.services.scheduler-core = {
      # the plan's last step makes the app passwords, then restarts Stalwart
      after = [
        "stalwart.service"
        "stalwart-apply-plan.service"
      ];
      # a Stalwart that is down at start keeps it restarting rather than failed
      unitConfig.StartLimitIntervalSec = 0;
      requires = [
        "scheduler-front.socket"
        "scheduler-admin.socket"
      ];
      serviceConfig = hardened // {
        User = "scheduler-core";
        Group = "nogroup";
        Sockets = [
          "scheduler-front.socket"
          "scheduler-admin.socket"
        ];
        ExecStart = "${lib.getExe cfg.package} core";
        StateDirectory = "scheduler";
        LoadCredential = "${policy.account.address}:${cfg.appPassword}";
        Restart = "on-failure";
        RestartSec = 5;
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
        IPAddressAllow = "localhost";
        IPAddressDeny = "any";
      };
    };
  };
}
