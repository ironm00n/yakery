# Published calendar feeds (as Outlook and iCloud publish them) kept in the group's one calendar by
# vdirsyncer, one way, and counted by the scheduler through the group: a moved event moves, a
# dropped one goes, an event both feeds carry stays until both drop it, an edit made in Stalwart
# is put back two syncs later, and a feed that can't be read keeps its last copy.
{ pkgs }:
let
  inherit (pkgs) lib;
  server = import ./scheduler-server.nix { inherit pkgs; };
  feed = {
    dir = "/var/lib/feed";
    port = 8000;
  };
  links = "/var/lib/test-feeds";
  names = [
    "outlook"
    "icloud"
  ];
in
pkgs.testers.runNixOSTest {
  name = "scheduler-feeds";

  defaults.virtualisation.graphics = false;

  nodes.server = {
    imports = [ server.node ];
    bundles.scheduler.feeds = lib.genAttrs names (name: "${links}/${name}");
    # the test runs each sync itself, once the links are written
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

    logins = sign_in(["owen", SCHEDULE, FEEDER])
    owen, owen_id = logins["owen"], account_ids["owen"]
    group_id = account_ids[GROUP]

    for name in ${builtins.toJSON names}:
        server.succeed(f"install -D -m 0400 /dev/null ${links}/{name} && printf %s http://127.0.0.1:${toString feed.port}/{name}.ics > ${links}/{name}")

    def publish(name, events):
        server.succeed(f"printf %s {shlex.quote(ics(events))} > ${feed.dir}/{name}.ics")

    def sync():
        code, _ = server.execute("systemctl start calendar-feeds")
        observe(f"sync.{len(observed)}", [code, server.succeed("journalctl -u calendar-feeds -o cat -n 8").strip()[-800:]])
        return code

    window = (utc(day(0) + " 00:00"), utc(day(7) + " 00:00"))

    def busy():
        got = jmap(logins[SCHEDULE], "Principal/getAvailability", {
            "accountId": account_ids[SCHEDULE], "id": group_id, "utcStart": window[0], "utcEnd": window[1]})
        return [[p["utcStart"], p["utcEnd"]] for p in got.get("list", [])]

    def hours(*locals):
        return merged([at(local, 60) for local in locals])

    o1, o2, i1, i2, both = f"{day(0)}T10:00", f"{day(1)}T10:00", f"{day(0)}T14:00", f"{day(3)}T10:00", f"{day(2)}T10:00"
    moved = f"{day(0)}T15:00"
    space = lambda local: local.replace("T", " ")

    with subtest("the first sync brings both feeds in, the event both carry once"):
        publish("outlook", [("o1", o1, 60), ("o2", o2, 60), ("both", both, 60)])
        publish("icloud", [("i1", i1, 60), ("both", both, 60)])
        expect("sync.first", sync() == 0)
        got = observe("busy.first", busy())
        expect("busy.first", got == hours(*map(space, [o1, o2, i1, both])), got)
        expect("contents.first", len(observe("contents.first", feeds_contents(logins[FEEDER]))) == 4)

    with subtest("a moved event moves, a dropped one goes, a new one comes"):
        publish("outlook", [("o1", moved, 60), ("both", both, 60)])
        publish("icloud", [("i1", i1, 60), ("both", both, 60), ("i2", i2, 60)])
        expect("sync.second", sync() == 0)
        got = observe("busy.second", busy())
        expect("busy.second", got == hours(*map(space, [moved, i1, both, i2])), got)

    with subtest("an event both feeds carry stays until both drop it"):
        publish("outlook", [("o1", moved, 60)])
        expect("sync.one-drops", sync() == 0)
        got = observe("busy.one-drops", busy())
        expect("busy.one-drops.kept", got == hours(*map(space, [moved, i1, both, i2])), got)
        publish("icloud", [("i1", i1, 60), ("i2", i2, 60)])
        expect("sync.both-drop", sync() == 0)
        got = observe("busy.both-drop", busy())
        expect("busy.both-drop.gone", got == hours(*map(space, [moved, i1, i2])), got)

    # vdirsyncer's revert of a change on the read-only side's behalf takes two runs: the first
    # records the edited copy's hash as the links' own, the second finds the file differs from it
    with subtest("an edit made in Stalwart is put back by the second sync after it"):
        href = feeds_contents(logins[FEEDER])[0]
        status, body = http(server, logins[FEEDER], "GET", href)
        edited = re.sub(r"DTSTART;TZID=([^:]+):(\d{8})T\d{6}", lambda m: f"DTSTART;TZID={m[1]}:{m[2]}T060000", body)
        status, _ = http(server, logins[FEEDER], "PUT", href, edited, "text/calendar")
        expect("edit.put", status == 204, status)
        edited_busy = observe("busy.edited", busy())
        expect("busy.edited.moved", edited_busy != hours(*map(space, [moved, i1, i2])), edited_busy)
        expect("sync.after-edit", sync() == 0)
        got = observe("busy.after-edit", busy())
        expect("busy.after-edit.kept", got == edited_busy, got)
        expect("sync.after-edit.again", sync() == 0)
        got = observe("busy.after-edit.again", busy())
        expect("busy.after-edit.again.restored", got == hours(*map(space, [moved, i1, i2])), got)

    with subtest("a feed that can't be read keeps its last copy"):
        server.succeed("mv ${feed.dir}/outlook.ics ${feed.dir}/outlook.gone")
        expect("sync.unreadable.fails", sync() != 0)
        got = observe("busy.unreadable", busy())
        expect("busy.unreadable.kept", got == hours(*map(space, [moved, i1, i2])), got)

    with subtest("the sync account reaches only the feeds' calendar"):
        status, _, hrefs = propfind(logins[FEEDER], HOME)
        observe("feeder.owner-home", [status, hrefs])
        expect("feeder.owner-home.refused", status in (403, 404) or hrefs == [], [status, hrefs])
        status, _, hrefs = propfind(logins[FEEDER], f"/dav/cal/{GROUP}%40${server.domain}/")
        observe("feeder.group-home", [status, hrefs])
        expect("feeder.group-home.feeds-only", [h for h in hrefs if h.count("/") > 4] == [FEEDS], hrefs)

    finish()
  '';
}
