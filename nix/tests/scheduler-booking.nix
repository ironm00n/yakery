# The scheduler end to end on Stalwart: the public week and a link's page through nginx, a
# booking and its invitation, the core killed at each crash point, two links racing for one
# slot, and the front's confinement.
{ pkgs }:
let
  inherit (pkgs) lib;
  server = import ./scheduler-server.nix { inherit pkgs; };
  inherit (server) domain;
  host = "schedule.${domain}";
  credentials = "/var/lib/test-app-passwords";
in
pkgs.testers.runNixOSTest {
  name = "scheduler-booking";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { config, ... }:
    {
      imports = [
        server.node
        ../../modules/scheduler.nix
      ];
      services.scheduler = {
        enable = true;
        owner = "owen@${domain}";
        inherit host;
        calendars = {
          work.label = "Work";
          personal.label = "Personal";
        };
        appPassword = name: "${credentials}/${name}";
        package = pkgs.callPackage ../../crates/scheduler/package.nix {
          inherit (config.services.scheduler) policy;
          crashPoints = true;
        };
      };
      services.nginx = {
        enable = true;
        virtualHosts.${host} = {
          locations."/".proxyPass = "http://127.0.0.1:8931";
          extraConfig = "access_log off;";
        };
      };
    };

  testScript = server.python + ''
    import hashlib
    import urllib.parse

    start_all()
    wait_for_stalwart()

    accounts = ["owen", "schedule", "schedule-work", "schedule-personal"]
    secrets = {n: app_password(n) for n in accounts}
    logins = {n: basic(f"{n}@${domain}", secrets[n]) for n in accounts}
    owen, owen_id = logins["owen"], account_ids["owen"]

    cal = calendars({
        "work": {"name": "Work", "timeZone": NY},
        "personal": {"name": "Personal", "timeZone": NY},
        "bookings": {"name": "Bookings", "timeZone": NY},
    })
    share(cal["work"], "schedule-work", FREE_BUSY)
    share(cal["personal"], "schedule-personal", FREE_BUSY)
    share(cal["bookings"], "schedule", BOOKINGS)
    made = create(owen, {
        "a": ev(cal["work"], "Alpha secret title", f"{day(0)}T10:00:00"),
        "b": ev(cal["personal"], "Bravo secret title", f"{day(1)}T13:00:00"),
    })
    assert not made.get("notCreated"), made

    for n in accounts[1:]:
        server.succeed(f"install -D -m 0400 /dev/null ${credentials}/{n} && printf %s {shlex.quote(secrets[n])} > ${credentials}/{n}")
    server.succeed("systemctl start scheduler-core")
    server.wait_for_unit("scheduler-core")

    def page(path, form=None):
        data = "" if form is None else "--data-raw " + shlex.quote(urllib.parse.urlencode(form))
        out = server.succeed(f"curl -s -w '\\n%{{http_code}}' -H 'Host: ${host}' {data} http://127.0.0.1{path}")
        body, status = out.rsplit("\n", 1)
        return int(status), body

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

    def at(local):
        return str(int(parse(utc(local)).replace(tzinfo=datetime.timezone.utc).timestamp()))

    booker = {"name": "Ada Lovelace", "address": "ada@remote.test", "note": "about the paper"}
    this_monday = today - datetime.timedelta(days=today.weekday())
    week = (monday - this_monday).days // 7
    window = f"--from {day(0)} --until {day(4)}"

    with subtest("the public week shows labelled blocks and no titles"):
        status, body = page(f"/?week={week}")
        expect("week.status", status == 200, status)
        expect("week.labels", ">Work<" in body and ">Personal<" in body)
        expect("week.no-titles", "secret title" not in body)
        expect("week.no-script", "<script" not in body)

    with subtest("a link offers free slots the gap away from busy time"):
        token = link(window, "--title 'Research chat'")
        offered = slots(token)
        observe("link.first-slots", offered[:3])
        expect("link.gap", at(f"{day(0)} 11:00") not in offered and at(f"{day(0)} 11:15") in offered)
        status, body = page(f"/b/{token}?slot={offered[0]}")
        expect("link.dialog", "<dialog open>" in body and "<script" not in body)
        status, body = page(f"/b/{'0' * 64}")
        expect("link.unknown", status == 404, status)

    with subtest("booking makes one event and one invitation"):
        status, body = page(f"/b/{token}", booker | {"slot": at(f"{day(0)} 11:15")})
        expect("book.status", status == 200 and "should reach ada@remote.test" in body, [status, body[-400:]])
        server.wait_until_succeeds("test -n \"$(ls /var/lib/sink/mail/new)\"", timeout=60)
        expect("book.event", len(events(token)) == 1)
        status, body = page(f"/b/{token}", booker | {"slot": at(f"{day(0)} 12:00")})
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
        status, _ = page(f"/b/{crashed}", booker | {"slot": at(local)})
        expect(f"crash.{at_step}.unanswered", status == 502, status)
        server.wait_until_succeeds(f"test -e /var/lib/scheduler/crashed-{at_step}")
        server.wait_for_unit("scheduler-core")
        status, _ = page(f"/b/{crashed}", booker | {"slot": at(local.replace(":00", ":30"))})
        observe(f"crash.{at_step}.another-slot", status)
        expect(f"crash.{at_step}.another-slot.refused", status in (202, 409), status)
        # a claim sent once before the crash is sent again only after the quiet period
        server.wait_until_succeeds(f"curl -s -H 'Host: ${host}' http://127.0.0.1/b/{crashed} | grep -q 'Booked for'", timeout=300)
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
        form = shlex.quote(urllib.parse.urlencode(booker | {"slot": at(f"{day(3)} 10:00")}))
        server.succeed(f"(curl -s -o /dev/null -w '%{{http_code}}' -H 'Host: ${host}' --data-raw {form} http://127.0.0.1/b/{down} > /tmp/down) >/dev/null 2>&1 &")
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
        expect("down.still-open", at(f"{day(3)} 10:00") in slots(down), slots(down)[:3])
        expect("down.no-event", events(down) == [], events(down))
        expect("down.no-invite", len(sink_mail()) == before, len(sink_mail()) - before)

    with subtest("two links posted at once for one slot make one event"):
        first, second = link(window), link(window)
        form = shlex.quote(urllib.parse.urlencode(booker | {"slot": at(f"{day(2)} 10:00")}))
        post = lambda t, out: f"curl -s -o /dev/null -w '%{{http_code}}' -H 'Host: ${host}' --data-raw {form} http://127.0.0.1/b/{t} > {out}"
        server.succeed(f"{post(first, '/tmp/r1')} & {post(second, '/tmp/r2')} & wait")
        statuses = sorted(server.succeed("cat /tmp/r1; echo; cat /tmp/r2").split())
        observe("race.statuses", statuses)
        expect("race.one-wins", statuses == ["200", "409"], statuses)
        expect("race.one-event", len(events(first)) + len(events(second)) == 1)

    with subtest("the front has no IP sockets of its own and no way to root's socket"):
        families = observe("front.families", server.succeed("systemctl show -P RestrictAddressFamilies scheduler-front").strip())
        expect("front.families.unix-only", families == "AF_UNIX", families)
        pid = server.succeed("systemctl show -P MainPID scheduler-front").strip()
        expect("front.seccomp", "Seccomp:\t2" in server.succeed(f"grep Seccomp: /proc/{pid}/status"))
        # the front can't be made to connect anywhere, so a stand-in runs as its user with its
        # families, beside a control without them
        reach = "systemd-run -q --wait --pipe -p User=scheduler-front {} curl -sS -o /dev/null http://127.0.0.1:8080/healthz/ready 2>&1"
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
