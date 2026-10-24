# The scheduler's two services and three sockets: the front serves nginx on a socket it is
# handed and can open no IP socket of its own; the core holds the app passwords and the links.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.scheduler;
  inherit (lib) mkOption types;
  accounts = import ../crates/scheduler/accounts.nix {
    inherit lib;
    calendars = lib.attrNames cfg.calendars;
  };
  policy = {
    inherit (cfg)
      owner
      host
      stalwart
      zone
      hours
      showCalendars
      ;
    notice = "PT${toString cfg.noticeHours}H";
    horizonWeeks = cfg.horizonWeeks;
    gap = "PT${toString cfg.gapMinutes}M";
    calendars = lib.mapAttrsToList (key: c: {
      inherit key;
      inherit (c) label;
    }) cfg.calendars;
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
    owner = mkOption { type = types.str; };
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
    calendars = mkOption {
      type = types.attrsOf (types.submodule { options.label = mkOption { type = types.str; }; });
    };
    showCalendars = mkOption {
      type = types.bool;
      default = true;
    };
    appPassword = mkOption {
      type = types.functionTo types.str;
      description = "The file holding an account's app password, by its local part.";
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
    assertions = [
      {
        assertion = cfg.calendars != { };
        message = "services.scheduler.calendars counts none of the owner's calendars, so every bookable hour would show free.";
      }
    ];
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
          FileDescriptorName = "http";
          Service = "scheduler-front.service";
        };
      };
      scheduler-front = {
        wantedBy = [ "sockets.target" ];
        listenStreams = [ "/run/scheduler/front.sock" ];
        socketConfig = {
          FileDescriptorName = "front";
          Service = "scheduler-core.service";
          SocketUser = "scheduler-core";
          SocketGroup = "scheduler-front";
          SocketMode = "0660";
        };
      };
      scheduler-admin = {
        wantedBy = [ "sockets.target" ];
        listenStreams = [ "/run/scheduler/admin.sock" ];
        socketConfig = {
          FileDescriptorName = "admin";
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
        LoadCredential = map (name: "${name}:${cfg.appPassword name}") (lib.attrNames accounts.core);
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
