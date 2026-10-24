{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  cfg = config.bundles.scheduler;
  inherit (my-lib.ironmoon) domain schedule mail;
  inherit (lib) mkOption types;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  address = "${ips.ipv6.prefix}::4";
  stalwart = "${mail.loopback.address}:${toString mail.loopback.port}";
  appPassword = local: config.services.stalwart.appPasswords."${local}@${domain}".file;
  accounts = import ../crates/scheduler/accounts.nix {
    inherit lib;
    calendars = lib.attrNames config.services.scheduler.calendars;
  };
  feed = accounts.feed.name;
in
{
  imports = [
    ../modules/scheduler.nix
    ../modules/calendar-feeds.nix
  ];

  options.bundles.scheduler = {
    enable = lib.mkEnableOption "busy times and one-time booking links at ${schedule.host}, on Stalwart; the calendars counted are {option}`services.scheduler.calendars`";
    owner = mkOption {
      type = types.str;
      description = "The account whose calendars are counted, whose Bookings calendar takes the bookings, and whose calendars the feeds are copied into.";
    };
    feeds = mkOption {
      type = types.attrsOf (
        types.submodule (
          { name, ... }:
          {
            options = {
              link = mkOption {
                type = types.str;
                description = "File holding the published calendar's link.";
              };
              label = mkOption { type = types.str; };
              calendar = mkOption {
                type = types.str;
                default = name;
                description = "The owner's calendar the feed is copied into, by its CalDAV path segment.";
              };
            };
          }
        )
      );
      default = { };
      description = "Published calendars (Outlook's ICS link, iCloud's public calendar), each counted as a calendar of its key.";
    };
  };

  config = lib.mkIf cfg.enable {
    services.scheduler = {
      enable = true;
      inherit (cfg) owner;
      inherit (schedule) host;
      inherit stalwart appPassword;
      zone = schedule.timeZone;
      calendars = lib.mapAttrs (_: f: { inherit (f) label; }) cfg.feeds;
    };

    services.calendar-feeds = {
      account = "${feed}@${domain}";
      password = appPassword feed;
      feeds = lib.mapAttrs (_: f: {
        inherit (f) link;
        calendar = "http://${stalwart}/dav/cal/${lib.escapeURL cfg.owner}/${f.calendar}/";
      }) cfg.feeds;
    };

    bundles.stalwart.programAccounts = lib.mapAttrs (_: permissions: { inherit permissions; }) (
      accounts.core // lib.optionalAttrs (cfg.feeds != { }) { ${feed} = accounts.feed.permissions; }
    );

    networking.interfaces.enp1s0.ipv6.addresses = [
      {
        inherit address;
        prefixLength = ips.ipv6.prefixLength;
      }
    ];

    services.nginx.appendHttpConfig = ''
      limit_req_zone $binary_remote_addr zone=schedule:1m rate=10r/s;
      map $request_method $schedule_booking {
        POST $binary_remote_addr;
        default "";
      }
      limit_req_zone $schedule_booking zone=schedule-booking:1m rate=3r/m;
    '';

    bundles.reverse-proxy = {
      enable = true;
      openFirewall = true;
      acme-email = "me@${domain}";
      hosts.${schedule.host} = {
        inherit (config.services.scheduler) port;
        listenAddresses = [
          ips.ipv4.address
          address
        ];
        # each link's token is in its path
        extraVhostConfig = "access_log off;";
        extraLocationConfig = ''
          limit_req zone=schedule burst=40 nodelay;
          limit_req zone=schedule-booking burst=5 nodelay;
        '';
      };
    };
  };
}
