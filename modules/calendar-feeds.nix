# Published calendar feeds (Outlook's ICS link, iCloud's public calendar) copied one way into one
# Stalwart calendar by vdirsyncer on a timer. A vdirsyncer pair owns its collection, so each feed
# is synced into its own directory and the calendar from links to all their files; an event in
# two feeds is one file, named by the hash of its content, kept while either has it.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.calendar-feeds;
  inherit (lib) mkOption types;
  state = "/var/lib/calendar-feeds";
  names = lib.attrNames cfg.feeds;
  feedDirs = map (name: "${state}/feeds/${name}") names;
  fetch = name: ''["command", "cat", "/run/credentials/calendar-feeds.service/${name}"]'';
  directory = name: ''
    type = "filesystem"
    path = "${state}/${name}/"
    fileext = ".ics"
  '';
  feed = name: ''
    [pair feed_${name}]
    a = "${name}_link"
    b = "${name}_files"
    collections = null
    conflict_resolution = "a wins"

    [storage ${name}_link]
    type = "http"
    url.fetch = ${fetch "feed-${name}"}

    [storage ${name}_files]
    ${directory "feeds/${name}"}
  '';
  vdirsyncer = pkgs.writeText "calendar-feeds.conf" ''
    [general]
    status_path = "${state}/status/"

    ${lib.concatMapStrings feed names}
    [pair calendar]
    a = "all"
    b = "stalwart"
    collections = null
    conflict_resolution = "a wins"
    partial_sync = "revert"

    [storage all]
    ${directory "all"}
    read_only = true

    [storage stalwart]
    type = "caldav"
    url = "${cfg.calendar}"
    username = "${cfg.account}"
    password.fetch = ${fetch "password"}
  '';
in
{
  options.services.calendar-feeds = {
    feeds = mkOption {
      type = types.attrsOf types.str;
      default = { };
      description = "Files holding each feed's link, by a name for it.";
    };
    calendar = mkOption {
      type = types.str;
      description = "CalDAV URL of the Stalwart calendar every feed is copied into.";
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
    assertions = [
      {
        assertion = lib.all (name: builtins.match "[a-z0-9-]+" name != null) names;
        message = "services.calendar-feeds.feeds must be named in [a-z0-9-], as vdirsyncer pairs and credentials are";
      }
    ];
    systemd.services.calendar-feeds = {
      after = [
        "stalwart.service"
        "stalwart-apply-plan.service"
      ];
      path = [ pkgs.vdirsyncer ];
      # a feed that can't be read keeps its last copy in the calendar
      script = ''
        mkdir -p ${toString feedDirs} ${state}/all
        vdirsyncer --config ${vdirsyncer} discover
        failed=0
        vdirsyncer --config ${vdirsyncer} sync ${toString (map (name: "feed_${name}") names)} || failed=1
        rm -rf ${state}/all.new && mkdir ${state}/all.new
        find ${toString feedDirs} -name '*.ics' -exec ln -sf -t ${state}/all.new {} +
        rm -rf ${state}/all && mv ${state}/all.new ${state}/all
        vdirsyncer --config ${vdirsyncer} sync calendar
        exit "$failed"
      '';
      serviceConfig = {
        Type = "oneshot";
        DynamicUser = true;
        StateDirectory = "calendar-feeds";
        LoadCredential = [
          "password:${cfg.password}"
        ]
        ++ lib.mapAttrsToList (name: link: "feed-${name}:${link}") cfg.feeds;
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
