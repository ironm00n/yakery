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
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  address = "${ips.ipv6.prefix}::4";
  stalwart = "${mail.loopback.address}:${toString mail.loopback.port}";
  accounts = lib.mapAttrs (_: a: a // { address = "${a.name}@${domain}"; }) (
    import ../crates/scheduler/accounts.nix
  );
  appPassword = account: config.services.stalwart.appPasswords.${account.address}.file;
  group = "calendars";
  holder = "${group}@${domain}";
  feeds = "feeds";
in
{
  imports = [
    ../modules/scheduler.nix
    ../modules/calendar-feeds.nix
  ];

  options.bundles.scheduler = {
    enable = lib.mkEnableOption "busy times and one-time booking links at ${schedule.host}, on Stalwart, for whoever shares a Bookings calendar with ${accounts.core.address}";
    feeds = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      description = "Files holding published calendars' links (Outlook's ICS link, iCloud's public calendar), by a name for each; all are copied into one calendar the scheduler counts.";
    };
    readers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Addresses the feeds' copy is shared with to read, so it shows in their calendar apps.";
    };
  };

  config = lib.mkIf cfg.enable {
    services.scheduler = {
      enable = true;
      account = accounts.core.address;
      appPassword = appPassword accounts.core;
      groups = [ holder ];
      inherit (schedule) host;
      inherit stalwart;
      zone = schedule.timeZone;
    };

    services.calendar-feeds = {
      inherit (cfg) feeds;
      account = accounts.feed.address;
      password = appPassword accounts.feed;
      calendar = "http://${stalwart}${my-lib.stalwart.calendarHref holder feeds}";
    };

    bundles.stalwart = {
      programAccounts = lib.mapAttrs' (
        _: a: lib.nameValuePair a.name { inherit (a) permissions; }
      ) accounts;
      groups = [ group ];
    };
    services.stalwart.calendars.${holder}.${feeds}.shares = {
      ${accounts.feed.address} = [
        "mayReadItems"
        "mayWriteAll"
      ];
      ${accounts.core.address} = [ "mayReadFreeBusy" ];
    }
    // lib.genAttrs cfg.readers (_: [
      "mayReadFreeBusy"
      "mayReadItems"
    ]);

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
