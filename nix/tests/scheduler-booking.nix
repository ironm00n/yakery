# The scheduler end to end on cx33-1's bundles: the public week and a link's page through nginx,
# busy time from the owner's calendars and the group's, a booking and its invitation, the core
# killed at each crash point, two links racing for one slot, a share wider than Bookings refused
# at start, and the front's confinement.
{ pkgs }:
let
  server = import ./scheduler-server.nix { inherit pkgs; };
  inherit (server) domain host;
in
pkgs.testers.runNixOSTest {
  name = "scheduler-booking";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { config, ... }:
    {
      imports = [ server.node ];
      services.scheduler.package = pkgs.callPackage ../../crates/scheduler/package.nix {
        inherit (config.services.scheduler) policy;
        crashPoints = true;
      };
    };

  testScript = server.python + ''
    import hashlib
    import urllib.parse

    start_all()
    wait_for_stalwart()

    logins = sign_in(["owen", FEEDER])
    owen, owen_id = logins["owen"], account_ids["owen"]

    cal = calendars({
        "work": {"name": "Work", "timeZone": NY},
        "personal": {"name": "Personal", "timeZone": NY},
        "bookings": {"name": "Bookings", "timeZone": NY},
    })
    share(cal["work"], SCHEDULE, FREE_BUSY)
    share(cal["personal"], SCHEDULE, FREE_BUSY)
    share(cal["bookings"], SCHEDULE, BOOKINGS)
    made = create(owen, {
        "a": ev(cal["work"], "Alpha secret title", f"{day(0)}T10:00:00"),
        "b": ev(cal["personal"], "Bravo secret title", f"{day(1)}T13:00:00"),
    })
    assert not made.get("notCreated"), made
    status, _ = http(server, logins[FEEDER], "PUT", FEEDS + "lecture.ics", ics([("lecture", f"{day(0)}T14:00", 60)]), "text/calendar")
    assert status == 201, status

    server.succeed("systemctl start scheduler-core")
    server.wait_for_unit("scheduler-core")

    def page(path, form=None):
        data = "" if form is None else "--data-raw " + shlex.quote(urllib.parse.urlencode(form))
        out = server.succeed(f"curl -s -w '\\n%{{http_code}}' {data} https://${host}{path}")
        body, status = out.rsplit("\n", 1)
        return int(status), body

    def post(token, form, out):
        return f"curl -s -o /dev/null -w '%{{http_code}}' --data-raw {shlex.quote(urllib.parse.urlencode(form))} https://${host}/b/{token} > {out}"

    def link(*args):
        url = server.succeed("scheduler link new --duration PT30M " + " ".join(args)).strip()
        observe("link.url-shape", re.sub("[0-9a-f]{64}", "<token>", url))
        return url.rsplit("/", 1)[1]

    def uid(token):
        return hashlib.sha256(bytes.fromhex(token)).hexdigest() + "@${host}"

    def events(token):
        return jmap(owen, "CalendarEvent/query", {"accountId": owen_id, "filter": {"uid": uid(token)}})["ids"]

    def slots(token):
        return re.findall(r'href="\?slot=(\d+)"', page(f"/b/{token}")[1])

    def epoch(local):
        return str(int(parse(utc(local)).replace(tzinfo=datetime.timezone.utc).timestamp()))

    booker = {"name": "Ada Lovelace", "address": "ada@remote.test", "note": "about the paper"}
    this_monday = today - datetime.timedelta(days=today.weekday())
    week = (monday - this_monday).days // 7
    window = f"--from {day(0)} --until {day(4)}"

    with subtest("the public week shows busy blocks and no titles"):
        status, body = page(f"/?week={week}")
        expect("week.status", status == 200, status)
        expect("week.busy", body.count(">busy<") >= 6, body.count(">busy<"))
        expect("week.no-titles", "secret title" not in body and "lecture" not in body.lower())
        expect("week.no-script", "<script" not in body)

    with subtest("a link offers free slots the gap away from the owner's and the group's busy time"):
        token = link(window, "--title 'Research chat'")
        offered = slots(token)
        observe("link.first-slots", offered[:3])
        expect("link.gap.owner", epoch(f"{day(0)} 11:00") not in offered and epoch(f"{day(0)} 11:15") in offered)
        expect("link.gap.group", epoch(f"{day(0)} 13:30") not in offered and epoch(f"{day(0)} 15:00") not in offered
               and epoch(f"{day(0)} 13:15") in offered and epoch(f"{day(0)} 15:15") in offered)
        status, body = page(f"/b/{token}?slot={offered[0]}")
        expect("link.dialog", "<dialog open>" in body and "<script" not in body)
        status, body = page(f"/b/{'0' * 64}")
        expect("link.unknown", status == 404, status)

    with subtest("booking makes one event and one invitation, from the owner"):
        status, body = page(f"/b/{token}", booker | {"slot": epoch(f"{day(0)} 11:15")})
        expect("book.status", status == 200 and "invitation from owen@${domain} should reach ada@remote.test" in body, [status, body[-400:]])
        server.wait_until_succeeds("test -n \"$(ls /var/lib/sink/mail/new)\"", timeout=60)
        expect("book.event", len(events(token)) == 1)
        status, body = page(f"/b/{token}", booker | {"slot": epoch(f"{day(0)} 12:00")})
        expect("book.again", status == 409, status)
        status, body = page(f"/b/{token}")
        expect("book.shows-time-only", "Booked for" in body and "Lovelace" not in body and "ada@remote.test" not in body)
        server.sleep(3)
        expect("book.one-invite", len(sink_mail()) == 1, sink_mail())

    def crash(at_step, local):
        server.succeed(
            "mkdir -p /run/systemd/system/scheduler-core.service.d"
            f" && printf '[Service]\\nEnvironment=SCHEDULER_CRASH={at_step}\\n' > /run/systemd/system/scheduler-core.service.d/crash.conf"
            " && systemctl daemon-reload && systemctl restart scheduler-core"
        )
        crashed = link(window)
        before = len(sink_mail())
        status, _ = page(f"/b/{crashed}", booker | {"slot": epoch(local)})
        expect(f"crash.{at_step}.unanswered", status == 502, status)
        server.wait_until_succeeds(f"test -e /var/lib/scheduler/crashed-{at_step}")
        server.wait_for_unit("scheduler-core")
        status, _ = page(f"/b/{crashed}", booker | {"slot": epoch(local.replace(":00", ":30"))})
        observe(f"crash.{at_step}.another-slot", status)
        expect(f"crash.{at_step}.another-slot.refused", status in (202, 409), status)
        # a claim sent once before the crash is sent again only after the quiet period
        server.wait_until_succeeds(f"curl -s https://${host}/b/{crashed} | grep -q 'Booked for'", timeout=300)
        expect(f"crash.{at_step}.one-event", len(events(crashed)) == 1, events(crashed))
        got = jmap(owen, "CalendarEvent/get", {"accountId": owen_id, "ids": events(crashed), "properties": ["start"]})["list"]
        expect(f"crash.{at_step}.slot-first-asked", got and got[0]["start"] == local.replace(" ", "T") + ":00", got)
        server.wait_until_succeeds(f"[ $(ls /var/lib/sink/mail/new | wc -l) -ge {before + 1} ]", timeout=60)
        server.sleep(3)
        expect(f"crash.{at_step}.one-invite", len(sink_mail()) == before + 1, len(sink_mail()) - before)
        observe(f"crash.{at_step}.restarts", server.succeed("systemctl show -P NRestarts scheduler-core").strip())

    with subtest("the core killed after writing the claim"):
        crash("claimed", f"{day(1)} 10:00")

    with subtest("the core killed after Stalwart made the event"):
        crash("created", f"{day(1)} 11:00")
        observe("links.list", server.succeed("scheduler link list"))
        server.succeed("rm /run/systemd/system/scheduler-core.service.d/crash.conf && systemctl daemon-reload && systemctl restart scheduler-core")

    with subtest("a booking asked while the core is down is refused, and never carried out"):
        down = link(window)
        before = len(sink_mail())
        queued = "ss -xlH src /run/scheduler/front.sock"
        server.succeed("systemctl stop stalwart scheduler-core")
        asked = post(down, booker | {"slot": epoch(f"{day(3)} 10:00")}, "/tmp/down")
        server.succeed(f"({asked}) >/dev/null 2>&1 &")
        server.wait_until_succeeds(f"{queued} | awk '$3 == 1 {{ok = 1}} END {{exit !ok}}'", timeout=30)
        observe("down.queued", server.succeed(queued).strip())
        server.wait_until_succeeds("test -s /tmp/down", timeout=90)
        status = server.succeed("cat /tmp/down").strip()
        expect("down.unavailable", status == "503", status)
        server.succeed("systemctl start stalwart")
        wait_for_stalwart()
        server.wait_until_succeeds(f"{queued} | awk '$3 == 0 {{ok = 1}} END {{exit !ok}}'", timeout=120)
        server.wait_for_unit("scheduler-core")
        server.sleep(5)
        expect("down.still-open", epoch(f"{day(3)} 10:00") in slots(down), slots(down)[:3])
        expect("down.no-event", events(down) == [], events(down))
        expect("down.no-invite", len(sink_mail()) == before, len(sink_mail()) - before)

    with subtest("two links posted at once for one slot make one event"):
        first, second = link(window), link(window)
        form = booker | {"slot": epoch(f"{day(2)} 10:00")}
        server.succeed(f"{post(first, form, '/tmp/r1')} & {post(second, form, '/tmp/r2')} & wait")
        statuses = sorted(server.succeed("cat /tmp/r1; echo; cat /tmp/r2").split())
        observe("race.statuses", statuses)
        expect("race.one-wins", statuses == ["200", "409"], statuses)
        expect("race.one-event", len(events(first)) + len(events(second)) == 1)

    with subtest("a share that lets schedule@ read the group stops the core at start"):
        group_id = account_ids[GROUP]
        feeds = next(c["id"] for c in jmap(admin, "Calendar/get", {"accountId": group_id})["list"] if c["name"] == "feeds")
        widen = lambda rights: jmap(admin, "Calendar/set", {"accountId": group_id, "update": {feeds: {"shareWith/" + account_ids[SCHEDULE]: rights}}})
        observe("widen.read", widen({"mayReadFreeBusy": True, "mayReadItems": True}))
        server.succeed("systemctl restart scheduler-core")
        server.wait_until_succeeds("journalctl -u scheduler-core -o cat | grep -q 'must see one account besides its own'", timeout=60)
        observe("widen.refusal", server.succeed("journalctl -u scheduler-core -o cat | grep 'must see one account' | tail -1").strip())
        widen(FREE_BUSY)
        server.wait_until_succeeds("curl -sf -o /dev/null https://${host}/", timeout=60)

    with subtest("the front has no IP sockets of its own and no way to root's socket"):
        families = observe("front.families", server.succeed("systemctl show -P RestrictAddressFamilies scheduler-front").strip())
        expect("front.families.unix-only", families == "AF_UNIX", families)
        pid = server.succeed("systemctl show -P MainPID scheduler-front").strip()
        expect("front.seccomp", "Seccomp:\t2" in server.succeed(f"grep Seccomp: /proc/{pid}/status"))
        # the front can't be made to connect anywhere, so a stand-in runs as its user with its
        # families, beside a control without them
        reach = "systemd-run -q --wait --pipe --collect -p User=scheduler-front {} curl -sS -o /dev/null http://127.0.0.1:8080/healthz/ready 2>&1"
        code, out = server.execute(reach.format("-p RestrictAddressFamilies=" + families))
        observe("front.reach-stalwart", [code, out.strip()[-200:]])
        expect("front.reach-stalwart.refused", code == 7, [code, out])
        code, out = server.execute(reach.format(""))
        expect("front.reach-stalwart.control", code == 0, [code, out])
        code, out = server.execute("runuser -u scheduler-front -- scheduler link list 2>&1")
        observe("front.admin-socket", [code, out.strip()[-200:]])
        expect("front.admin-socket.refused", code != 0 and "Permission denied" in out, out)
        code, out = server.execute("runuser -u nobody -- curl -sS --unix-socket /run/scheduler/front.sock http://x/ 2>&1")
        observe("nobody.front-socket", [code, out.strip()[-200:]])
        expect("nobody.front-socket.refused", code == 7, out)

    with subtest("nginx keeps no log of the links"):
        code, _ = server.execute("grep -rq '/b/' /var/log/nginx")
        expect("nginx.no-log", code != 0)

    finish()
  '';
}
