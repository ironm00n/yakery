# A published calendar feed (as Outlook and iCloud publish one) kept in a Stalwart calendar by
# vdirsyncer, one way, so the scheduler counts it like any other: a moved event moves, a
# dropped one goes, and an edit made in Stalwart stays until the feed changes that event.
{ pkgs }:
let
  inherit (pkgs) lib;
  server = import ./scheduler-server.nix { inherit pkgs; };
  inherit (server) domain;
  feeder =
    (import ../../crates/scheduler/accounts.nix {
      inherit lib;
      calendars = [ ];
    }).feed.name;
  feed = {
    dir = "/var/lib/feed";
    port = 8000;
  };
  calendar = "/dav/cal/owen%40${domain}/outlook/";
in
pkgs.testers.runNixOSTest {
  name = "scheduler-feeds";

  defaults.virtualisation.graphics = false;

  nodes.server = {
    imports = [
      server.node
      ../../modules/calendar-feeds.nix
    ];
    services.calendar-feeds = {
      account = "${feeder}@${domain}";
      password = "/var/lib/test-app-passwords/${feeder}";
      feeds.outlook = {
        link = "/var/lib/test-feeds/outlook";
        calendar = "http://127.0.0.1:8080${calendar}";
      };
    };
    # the test runs each sync itself, once the app password exists
    systemd.timers.calendar-feeds.wantedBy = lib.mkForce [ ];
    systemd.services.feed = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig.StateDirectory = baseNameOf feed.dir;
      script = "${lib.getExe pkgs.python3} -m http.server ${toString feed.port} --bind 127.0.0.1 --directory ${feed.dir}";
    };
  };

  testScript = server.python + ''
    start_all()
    wait_for_stalwart()

    accounts = ["owen", "schedule-outlook", "${feeder}"]
    secrets = {n: app_password(n) for n in accounts}
    logins = {n: basic(f"{n}@${domain}", secrets[n]) for n in accounts}
    owen, owen_id = logins["owen"], account_ids["owen"]

    made = '<C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:set><D:prop><D:displayname>Outlook</D:displayname></D:prop></D:set></C:mkcalendar>'
    status, _ = http(server, owen, "MKCALENDAR", "${calendar}", made, "application/xml")
    expect("calendar.made", status == 201, status)
    cal = calendars({"work": {"name": "Work", "timeZone": NY}})
    cal["outlook"] = next(c["id"] for c in jmap(owen, "Calendar/get", {"accountId": owen_id})["list"] if c["name"] == "Outlook")
    share(cal["outlook"], "schedule-outlook", FREE_BUSY)
    share(cal["outlook"], "${feeder}", {"mayReadItems": True, "mayWriteAll": True})

    def ics(events):
        lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//test//feed//EN"]
        for uid, (day_, start, sequence) in events.items():
            begin = (datetime.datetime.fromisoformat(f"{day_}T{start}")).strftime("%Y%m%dT%H%M%S")
            lines += ["BEGIN:VEVENT", f"UID:{uid}", "DTSTAMP:20261001T000000Z", f"SEQUENCE:{sequence}",
                      f"DTSTART;TZID={NY}:{begin}", "DURATION:PT1H", f"SUMMARY:Feed {uid}", "END:VEVENT"]
        return "\r\n".join(lines + ["END:VCALENDAR", ""])

    def publish(events):
        server.succeed(f"printf %s {shlex.quote(ics(events))} > ${feed.dir}/outlook.ics")

    for path, text in [("/var/lib/test-app-passwords/${feeder}", secrets["${feeder}"]),
                       ("/var/lib/test-feeds/outlook", "http://127.0.0.1:${toString feed.port}/outlook.ics")]:
        server.succeed(f"install -D -m 0400 /dev/null {path} && printf %s {shlex.quote(text)} > {path}")

    def sync():
        code, _ = server.execute("systemctl start calendar-feeds")
        observe(f"sync.{len(observed)}", server.succeed("journalctl -u calendar-feeds -o cat -n 6").strip()[-400:])
        return code

    def busy():
        got = jmap(logins["schedule-outlook"], "Principal/getAvailability", {
            "accountId": account_ids["schedule-outlook"], "id": owen_id,
            "utcStart": utc(day(0) + " 00:00"), "utcEnd": utc(day(7) + " 00:00")})
        return [[p["utcStart"], p["utcEnd"]] for p in got.get("list", [])]

    def hour(local):
        start = utc(local)
        return [start, later(start, 60)]

    with subtest("the first sync brings the feed in"):
        publish({"e1": (day(0), "10:00", 0), "e2": (day(1), "10:00", 0)})
        expect("sync.first", sync() == 0)
        got = observe("busy.first", busy())
        expect("busy.first", got == [hour(f"{day(0)} 10:00"), hour(f"{day(1)} 10:00")], got)

    with subtest("a moved event moves, a dropped one goes, a new one comes"):
        publish({"e1": (day(0), "14:00", 1), "e3": (day(2), "10:00", 0)})
        expect("sync.second", sync() == 0)
        got = observe("busy.second", busy())
        expect("busy.second", got == [hour(f"{day(0)} 14:00"), hour(f"{day(2)} 10:00")], got)

    with subtest("events land under the hash of their content as their UID"):
        found = jmap(owen, "CalendarEvent/query", {"accountId": owen_id}).get("ids", [])
        events = observe("feed.events", jmap(owen, "CalendarEvent/get", {"accountId": owen_id, "ids": found, "properties": ["uid", "start", "calendarIds"]}).get("list", []))
        expect("feed.hashed-uids", sorted(e["uid"] for e in events) != ["e1", "e3"] and all(len(e["uid"]) == 64 for e in events), events)

    with subtest("an edit made in Stalwart stays until the feed changes that event"):
        e3 = next(e["id"] for e in events if e["start"] == f"{day(2)}T10:00:00")
        jmap(owen, "CalendarEvent/set", {"accountId": owen_id, "update": {e3: {"start": f"{day(2)}T16:00:00"}}})
        expect("sync.third", sync() == 0)
        got = observe("busy.third", busy())
        expect("busy.third.kept", got == [hour(f"{day(0)} 14:00"), hour(f"{day(2)} 16:00")], got)
        publish({"e1": (day(0), "14:00", 1), "e3": (day(2), "10:00", 1)})
        expect("sync.fourth", sync() == 0)
        got = observe("busy.fourth", busy())
        expect("busy.fourth.restored", got == [hour(f"{day(0)} 14:00"), hour(f"{day(2)} 10:00")], got)

    with subtest("the sync account reaches only the calendar shared with it"):
        status, _, _ = propfind(logins["${feeder}"], calendar_href("Work"))
        expect("feed.work.refused", status in (403, 404), status)
        status, _, hrefs = propfind(logins["${feeder}"], HOME)
        observe("feed.home", [status, hrefs])
        expect("feed.home.outlook-only", calendar_href("Work") not in hrefs, hrefs)

    finish()
  '';
}
