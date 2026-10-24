# Published calendar feeds (Outlook's ICS link, iCloud's public calendar) copied one way into
# Stalwart calendars by vdirsyncer on a timer, as one program account that may write only the
# calendars shared with it. Each feed link is a secret, read from a file by LoadCredential.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.calendar-feeds;
  inherit (lib) mkOption types;
  credential = name: "/run/credentials/calendar-feeds.service/${name}";
  fetch = name: ''["command", "cat", "${credential name}"]'';
  pair = name: feed: ''
    [pair ${name}]
    a = "${name}_feed"
    b = "${name}_stalwart"
    collections = null
    conflict_resolution = "a wins"
    partial_sync = "revert"

    [storage ${name}_feed]
    type = "http"
    url.fetch = ${fetch "feed-${name}"}

    [storage ${name}_stalwart]
    type = "caldav"
    url = "${feed.calendar}"
    username = "${cfg.account}"
    password.fetch = ${fetch "password"}
  '';
  vdirsyncer = pkgs.writeText "calendar-feeds.conf" ''
    [general]
    status_path = "/var/lib/calendar-feeds/status/"

    ${lib.concatStrings (lib.mapAttrsToList pair cfg.feeds)}
  '';
in
{
  options.services.calendar-feeds = {
    feeds = mkOption {
      type = types.attrsOf (
        types.submodule {
          options = {
            link = mkOption {
              type = types.str;
              description = "File holding the feed's link.";
            };
            calendar = mkOption {
              type = types.str;
              description = "CalDAV URL of the Stalwart calendar the feed is copied into.";
            };
          };
        }
      );
      default = { };
    };
    account = mkOption { type = types.str; };
    password = mkOption {
      type = types.str;
      description = "File holding the account's app password.";
    };
    every = mkOption {
      type = types.str;
      default = "15min";
    };
  };

  config = lib.mkIf (cfg.feeds != { }) {
    systemd.services.calendar-feeds = {
      after = [
        "stalwart.service"
        "stalwart-apply-plan.service"
      ];
      path = [ pkgs.vdirsyncer ];
      script = ''
        vdirsyncer --config ${vdirsyncer} discover
        vdirsyncer --config ${vdirsyncer} sync
      '';
      serviceConfig = {
        Type = "oneshot";
        DynamicUser = true;
        StateDirectory = "calendar-feeds";
        LoadCredential = [
          "password:${cfg.password}"
        ]
        ++ lib.mapAttrsToList (name: feed: "feed-${name}:${feed.link}") cfg.feeds;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
        ];
        UMask = "0077";
      };
    };
    systemd.timers.calendar-feeds = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "1min";
        OnUnitActiveSec = cfg.every;
      };
    };
  };
}
