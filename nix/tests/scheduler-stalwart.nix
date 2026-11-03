# What the scheduler's design rests on in Stalwart, run rather than read: free/busy-only shares
# seen through Principal/getAvailability, a member-less group's calendar reached by share alone,
# and a create-only Bookings share whose events Stalwart sends invitations for.
{ pkgs }:
let
  server = import ./scheduler-server.nix { inherit pkgs; };
  inherit (server) domain accounts;
in
pkgs.testers.runNixOSTest {
  name = "scheduler-stalwart";

  defaults.virtualisation.graphics = false;

  nodes.server = server.node;
  nodes.client = server.client;

  testScript = server.python + ''
    names = ["owen", SCHEDULE, FEEDER, "schedule-nosend", "schedule-probe"]
    start_all()
    wait_for_stalwart()

    logins = sign_in(names)
    owen, owen_id = logins["owen"], account_ids["owen"]
    schedule, schedule_id = logins[SCHEDULE], account_ids[SCHEDULE]

    with subtest("each program account's session and permissions"):
        for n in names[1:]:
            status, s = session(server, logins[n])
            observe(f"session.{n}.accounts", sorted(s["accounts"]) if s else status)
            status, payload = http(server, logins[n], "GET", "/api/account")
            observe(f"api-account.{n}", json.loads(payload).get("permissions") if status == 200 else status)
        listed = set(observed[f"api-account.{SCHEDULE}"])
        declared = set(${builtins.toJSON accounts.core.permissions})
        expect("api-account.schedule.within-declared", listed <= declared, sorted(listed ^ declared))
        expect("api-account.schedule.hides-principal", not any(p.startswith("jmapPrincipal") for p in listed), sorted(listed))
        observe("session.capabilities", sorted(session(server, owen)[1]["capabilities"]))

    with subtest("the owner's calendars"):
        cal = calendars({
            "work": {"name": "Work", "timeZone": NY},
            "personal": {"name": "Personal"},
            "other": {"name": "Other", "timeZone": NY},
            "bookings": {"name": "Bookings", "timeZone": NY},
        })
        observe("calendar.ids", cal)
        got = jmap(owen, "Calendar/get", {"accountId": owen_id})
        observe("calendars.after", [{k: c.get(k) for k in ("id", "name", "timeZone", "includeInAvailability", "isSubscribed")} for c in got["list"]])

    window = (utc(day(0) + " 00:00"), utc(day(7) + " 00:00"))
    observe("window", window)

    with subtest("events, Other's first so they take the low ids"):
        other = {f"o{i}": ev(cal["other"], f"Other {i}", f"{day(4)}T{9 + i:02}:00:00", 30) for i in range(6)}
        made = create(owen, other)
        expect("other.created", not made.get("notCreated"), made.get("notCreated"))
        ids = {k: v["id"] for k, v in made["created"].items()}
        events = {
            "w-timed": ev(cal["work"], "Timed", f"{day(0)}T10:00:00"),
            "w-secret": ev(cal["work"], "Secret", f"{day(0)}T12:00:00", privacy="secret"),
            "w-private": ev(cal["work"], "Private", f"{day(0)}T13:30:00", privacy="private"),
            "w-free": ev(cal["work"], "Free", f"{day(0)}T15:00:00", freeBusyStatus="free"),
            "w-cancelled": ev(cal["work"], "Cancelled", f"{day(0)}T16:30:00", status="cancelled"),
            "w-weekly": ev(cal["work"], "Weekly", f"{day(1)}T09:00:00", recurrenceRule={"frequency": "weekly", "count": 4}),
            "w-floating": ev(cal["work"], "Floating", f"{day(2)}T10:00:00", tz=None),
            "w-allday": ev(cal["work"], "All day", f"{day(3)}T00:00:00", tz=None, showWithoutTime=True) | {"duration": "P1D"},
            "w-old": ev(cal["work"], "Old daily", (monday - datetime.timedelta(days=3100)).isoformat() + "T08:00:00", 30, recurrenceRule={"frequency": "daily"}),
            "w-recent": ev(cal["work"], "Recent daily", (monday - datetime.timedelta(days=2900)).isoformat() + "T07:00:00", 30, recurrenceRule={"frequency": "daily"}),
            "p-floating": ev(cal["personal"], "Floating", f"{day(2)}T10:00:00", tz=None),
            "p-allday": ev(cal["personal"], "All day", f"{day(3)}T00:00:00", tz=None, showWithoutTime=True) | {"duration": "P1D"},
            "p-timed": ev(cal["personal"], "Timed", f"{day(0)}T18:00:00"),
        }
        made = create(owen, events)
        expect("events.created", not made.get("notCreated"), made.get("notCreated"))
        ids |= {k: v["id"] for k, v in made["created"].items()}
        observe("event.ids", ids)

    def availability(name, principal=owen_id, **extra):
        result = jmap(logins[name], "Principal/getAvailability", {
            "accountId": account_ids[name], "id": principal, "utcStart": window[0], "utcEnd": window[1], **extra})
        if "list" not in result:
            return result
        return [[p["utcStart"], p["utcEnd"]] for p in result["list"]]

    recent = [at(f"{day(n)} 07:00", 30) for n in range(7)]
    collisions = {k: [o for o in other if ids[o] == cal[k]] for k in cal}
    observe("collisions.predicted", collisions)
    other_at = {f"o{i}": at(f"{day(4)} {9 + i:02}:00", 30) for i in range(6)}
    collided = lambda *keys: [other_at[o] for k in keys for o in collisions[k]]
    work = [at(f"{day(0)} 10:00", 60), at(f"{day(0)} 13:30", 60), at(f"{day(1)} 09:00", 60),
            at(f"{day(2)} 10:00", 60), (utc(day(3) + " 00:00"), utc(day(4) + " 00:00"))] + recent
    personal = [(f"{day(2)}T10:00:00Z", f"{day(2)}T11:00:00Z"), (f"{day(3)}T00:00:00Z", f"{day(4)}T00:00:00Z"),
                at(f"{day(0)} 18:00", 60)]
    expected = merged(work + personal + collided("work", "personal", "bookings"))

    with subtest("no share yet: schedule@ sees the owner free"):
        got = observe("availability.unshared", availability(SCHEDULE))
        expect("unshared.empty", got == [], got)

    with subtest("free/busy shares and Bookings show schedule@ the owner's account alone"):
        share(cal["work"], SCHEDULE, FREE_BUSY)
        share(cal["personal"], SCHEDULE, FREE_BUSY)
        share(cal["bookings"], SCHEDULE, BOOKINGS)
        status, s = session(server, schedule)
        seen = observe("session.shared.schedule", sorted(s["accounts"]))
        expect("session.shared.schedule.owner-only", seen == sorted([schedule_id, owen_id]), seen)

    with subtest("schedule@'s availability: every shared calendar, merged"):
        got = observe("availability.schedule", availability(SCHEDULE))
        expect("availability.schedule", got == expected, {"want": expected})

    with subtest("includeInAvailability none on Personal counts nothing of its own"):
        jmap(owen, "Calendar/set", {"accountId": owen_id, "update": {cal["personal"]: {"includeInAvailability": "none"}}})
        got = observe("availability.none", availability(SCHEDULE))
        want = merged(work + collided("work", "personal", "bookings"))
        expect("none.only-collisions", got == want, {"want": want})
        jmap(owen, "Calendar/set", {"accountId": owen_id, "update": {cal["personal"]: {"includeInAvailability": "all"}}})
        expect("all.again", availability(SCHEDULE) == expected)

    state = jmap(owen, "CalendarEvent/get", {"accountId": owen_id, "ids": []})["state"]

    with subtest("schedule@ reads Bookings and no other calendar, and no event"):
        got = observe("schedule.Calendar/get", jmap(schedule, "Calendar/get", {"accountId": owen_id}))
        expect("schedule.one-calendar", [c["id"] for c in got.get("list", [])] == [cal["bookings"]], got)
        for method, args in [
            ("CalendarEvent/get", {"ids": [ids["w-timed"], ids["p-timed"]]}),
            ("CalendarEvent/query", {}),
            ("CalendarEvent/changes", {"sinceState": state}),
            ("Principal/get", {"ids": [owen_id]}),
        ]:
            got = observe(f"read.schedule.{method}", jmap(schedule, method, {"accountId": owen_id, **args}))
            expect(f"read.schedule.{method}.refused", forbidden(got), got)

    with subtest("a free/busy share confines an account that holds every read permission"):
        share(cal["work"], "schedule-probe", FREE_BUSY)
        for method, args in [
            ("Calendar/get", {}),
            ("CalendarEvent/get", {"ids": [ids["w-timed"]]}),
            ("CalendarEvent/query", {}),
            ("CalendarEvent/changes", {"sinceState": state}),
        ]:
            got = observe(f"read.schedule-probe.{method}", jmap(logins["schedule-probe"], method, {"accountId": owen_id, **args}))
            expect(f"read.schedule-probe.{method}.refused", forbidden(got), got)
        got = observe("availability.schedule-probe", availability("schedule-probe"))
        want = merged(work + collided("work"))
        expect("availability.schedule-probe", got == want, {"want": want})

    with subtest("what eventProperties adds for a free/busy sharee"):
        got = jmap(schedule, "Principal/getAvailability", {
            "accountId": schedule_id, "id": owen_id, "utcStart": window[0], "utcEnd": window[1],
            "eventProperties": ["id", "baseEventId"]})
        listed = [sorted((p.get("event") or {}).keys()) for p in got.get("list", [])]
        observe("event-properties.schedule", {"periods": len(listed), "keys": sorted(set(map(tuple, listed)))})
        expect("event-properties.ids-only", len(listed) > len(expected) and all(k == ["baseEventId", "id"] for k in listed), listed)

    with subtest("CalDAV"):
        work_href = calendar_href("Work")
        status, body, hrefs = propfind(owen, work_href)
        event_href = next(h for h in hrefs if h != work_href)
        observe("dav.owner.work", [work_href, event_href])
        for n in [SCHEDULE, "schedule-probe"]:
            for method, path in [("GET", event_href), ("PROPFIND", work_href), ("PROPFIND", HOME)]:
                status, body = http(server, logins[n], method, path, headers=["Depth: 1"])
                observe(f"dav.{n}.{method}.{path}", [status, body[:200]])
                expect(f"dav.{n}.{method}.{path}.refused", status == 403, status)

    with subtest("the group's calendar, made by the plan and reached by share alone"):
        group_id = account_ids[GROUP]
        status, s = session(server, schedule)
        expect("group.not-in-session", group_id not in s["accounts"], sorted(s["accounts"]))
        found = observe("group.query", jmap(schedule, "Principal/query", {"accountId": schedule_id, "filter": {"email": f"{GROUP}@${domain}"}}))
        expect("group.found-by-address", found.get("ids") == [group_id], found)
        status, _ = http(server, logins[FEEDER], "PUT", FEEDS + "lecture.ics", ics([("lecture", f"{day(5)}T10:00", 60)]), "text/calendar")
        expect("group.feeder-writes", status == 201, status)
        held = observe("group.calendars", [[c["id"], c["name"]] for c in jmap(admin, "Calendar/get", {"accountId": group_id})["list"]])
        feeds_id = next(i for i, n in held if n == "feeds")
        unshared = next(i for i, n in held if n != "feeds")
        hours_at = {f"u{h}": f"{day(5)} {h}:00" for h in (12, 14, 16)}
        made = jmap(admin, "CalendarEvent/set", {"accountId": group_id, "create": {
            k: ev(unshared, "Unshared", local.replace(" ", "T") + ":00") for k, local in hours_at.items()}})
        expect("group.unshared-events", sorted(made.get("created", {})) == sorted(hours_at), made)
        collided_group = observe("group.collisions.predicted", [k for k, v in made.get("created", {}).items() if v["id"] == feeds_id])
        got = observe("availability.group", availability(SCHEDULE, group_id))
        want = merged([at(f"{day(5)} 10:00", 60)] + [at(hours_at[k], 60) for k in collided_group])
        expect("group.counts-its-shared-calendar-and-collisions", got == want, {"want": want})
        hrefs = propfind(owen, HOME)[2]
        expect("group.in-reader-home", FEEDS in hrefs, hrefs)
        status, _ = http(server, owen, "PUT", FEEDS + "owner.ics", ics([("owner", f"{day(5)}T16:00", 60)]), "text/calendar")
        expect("group.reader-cannot-write", status == 403, status)
        got = availability(SCHEDULE)
        expect("group.not-counted-under-the-reader", got == expected, {"want": expected})

    uid = "0" * 64 + "@schedule.${domain}"
    booking = ev(cal["bookings"], "Research chat with Ada", f"{day(2)}T15:00:00", 30) | {
        "uid": uid,
        "description": "Booked with link 00000000",
        "organizerCalendarAddress": "mailto:owen@${domain}",
        "participants": {
            "owner": {"calendarAddress": "mailto:owen@${domain}", "roles": {"owner": True, "attendee": True}, "participationStatus": "accepted"},
            "booker": {"name": "Ada Lovelace", "calendarAddress": "mailto:ada@remote.test", "roles": {"attendee": True}, "expectReply": True},
        },
    }

    with subtest("schedule@ books, and Stalwart sends the invitation"):
        got = observe("book.first", create(schedule, {"b": booking}, sendSchedulingMessages=True))
        expect("book.created", "b" in got.get("created", {}), got)
        booked = got.get("created", {}).get("b", {}).get("id")
        server.wait_until_succeeds("test -n \"$(ls /var/lib/sink/mail/new)\"", timeout=60)
        server.sleep(5)
        files = sink_mail()
        expect("sink.one-mail", len(files) == 1, files)
        msg = sink_message(files[0])
        parts = [(p.get_content_type(), p.get_param("method")) for p in msg.walk()]
        headers = observe("sink.headers", {k: msg[k] for k in ("From", "To", "Reply-To", "Subject", "Sender")})
        expect("sink.from-owner", msg["From"].addresses[0].addr_spec == "owen@${domain}" and msg["Reply-To"] == "owen@${domain}", headers)
        expect("sink.to-booker", [a.addr_spec for a in msg["To"].addresses] == ["ada@remote.test"], headers)
        observe("sink.parts", parts)
        ics_part = next((p.get_content() for p in msg.walk() if p.get_content_type() == "text/calendar"), "")
        observe("sink.ics", [l for l in ics_part.splitlines() if l.split(":")[0].split(";")[0] in ("METHOD", "ORGANIZER", "ATTENDEE", "UID", "DTSTART", "SUMMARY")])
        expect("sink.request", ("text/calendar", "REQUEST") in parts, parts)
        html = next((p.get_content() for p in msg.walk() if p.get_content_type() == "text/html"), "")
        observe("sink.rsvp-links", sorted(set(re.findall(r"https?://[^\"' ]*rsvp[^\"' ]*", html)))[:3])

    with subtest("the owner's view of the booking"):
        got = jmap(owen, "CalendarEvent/get", {"accountId": owen_id, "ids": [booked], "properties": ["calendarIds", "uid", "organizerCalendarAddress", "participants", "title", "start", "timeZone"]})
        observe("book.owner-view", got)

    with subtest("a second create with the same UID"):
        got = observe("book.again", create(schedule, {"b": booking}, sendSchedulingMessages=True))
        refusal = got.get("notCreated", {}).get("b", {})
        expect("book.again.uid-taken", refusal.get("type") == "invalidProperties" and refusal.get("properties") == ["uid"], got)
        server.sleep(5)
        expect("book.again.no-mail", len(sink_mail()) == 1, sink_mail())

    with subtest("what schedule@ may not do"):
        tries = {
            "update": jmap(schedule, "CalendarEvent/set", {"accountId": owen_id, "update": {booked: {"title": "changed"}}}),
            "destroy": jmap(schedule, "CalendarEvent/set", {"accountId": owen_id, "destroy": [booked]}),
            "create-in-work": create(schedule, {"w": ev(cal["work"], "Intruder", f"{day(2)}T16:00:00")}),
            "submission": jmap(schedule, "EmailSubmission/set", {"accountId": schedule_id, "create": {"s": {"emailId": "x", "identityId": "y"}}}, MAIL),
        }
        for k, got in tries.items():
            observe(f"schedule.{k}", got)
        for k in ["update", "destroy", "submission"]:
            expect(f"schedule.{k}.forbidden", forbidden(tries[k]), tries[k])
        work_try = tries["create-in-work"].get("notCreated", {}).get("w", {})
        expect("schedule.create-in-work.forbidden", work_try.get("type") == "forbidden", tries["create-in-work"])

    with subtest("organizer Stalwart picks when none is named"):
        bare = ev(cal["bookings"], "No organizer", f"{day(2)}T17:00:00", 30) | {"participants": {
            "booker": {"calendarAddress": "mailto:bob@remote.test", "roles": {"attendee": True}}}}
        got = create(schedule, {"n": bare})
        made = got.get("created", {}).get("n", {}).get("id")
        observe("organizer.default", jmap(owen, "CalendarEvent/get", {"accountId": owen_id, "ids": [made], "properties": ["organizerCalendarAddress", "participants"]}) if made else got)

    with subtest("schedule@'s availability includes the booking"):
        got = observe("availability.schedule.after-booking", availability(SCHEDULE))
        expect("availability.with-booking", list(at(f"{day(2)} 15:00", 30)) in got, got)

    with subtest("a write-only share is invisible yet writable"):
        share(cal["other"], SCHEDULE, {"mayWriteAll": True})
        listed = observe("writeonly.Calendar/get", [c["id"] for c in jmap(schedule, "Calendar/get", {"accountId": owen_id}).get("list", [])])
        expect("writeonly.invisible", listed == [cal["bookings"]], listed)
        got = observe("writeonly.create-in-other", create(schedule, {"w": ev(cal["other"], "Write-only", f"{day(2)}T16:00:00")}))
        expect("writeonly.writable", "w" in got.get("created", {}), got)
        share(cal["other"], SCHEDULE, None)

    with subtest("four creates with one UID at once, twenty times"):
        auth = shlex.quote("Authorization: " + schedule)
        counts = []
        for round in range(20):
            twin = booking | {"uid": f"{round:064}@twin.${domain}", "start": f"{day(5)}T{8 + round // 4:02}:{15 * (round % 4):02}:00", "participants": {}}
            twin.pop("organizerCalendarAddress")
            body = shlex.quote(json.dumps({"using": CAL, "methodCalls": [["CalendarEvent/set", {"accountId": owen_id, "create": {"t": twin}}, "0"]]}))
            cmd = f"curl -s -o /dev/null -H {auth} -H 'Content-Type: application/json' --data-binary {body} http://127.0.0.1:8080/jmap/"
            server.succeed(f"{cmd} & {cmd} & {cmd} & {cmd} & wait")
            counts.append(len(jmap(owen, "CalendarEvent/query", {"accountId": owen_id, "filter": {"uid": twin["uid"]}}).get("ids", [])))
        observe("twin.events-per-uid", counts)

    with subtest("without calendarSchedulingSend"):
        share(cal["bookings"], "schedule-nosend", BOOKINGS)
        before = len(sink_mail())
        quiet = booking | {"uid": "2" * 64 + "@schedule.${domain}", "start": f"{day(3)}T16:00:00"}
        asking = observe("nosend.asking", create(logins["schedule-nosend"], {"q": quiet}, sendSchedulingMessages=True))
        expect("nosend.asking.refused", asking.get("notCreated", {}).get("q", {}).get("type") == "forbidden", asking)
        plain = observe("nosend.not-asking", create(logins["schedule-nosend"], {"q": quiet}))
        expect("nosend.not-asking.created", "q" in plain.get("created", {}), plain)
        server.sleep(5)
        expect("nosend.no-mail", len(sink_mail()) == before, len(sink_mail()) - before)

    with subtest("what a sharee sees of the owner's settings"):
        jmap(owen, "Calendar/set", {"accountId": owen_id, "update": {cal["bookings"]: {"includeInAvailability": "attending"}}})
        mine = jmap(owen, "Calendar/get", {"accountId": owen_id, "ids": [cal["bookings"]]})["list"][0]
        theirs = jmap(schedule, "Calendar/get", {"accountId": owen_id, "ids": [cal["bookings"]]})["list"][0]
        observe("sharee.view", {k: [mine.get(k), theirs.get(k)] for k in ("includeInAvailability", "timeZone", "isSubscribed")})
        jmap(owen, "Calendar/set", {"accountId": owen_id, "update": {cal["bookings"]: {"includeInAvailability": "all"}}})

    with subtest("refusals: another host, a wrong secret"):
        status, _ = session(client, schedule)
        observe("remote.session", status)
        expect("remote.refused", status in (401, 403), status)
        status, _ = session(server, basic(f"{SCHEDULE}@${domain}", "wrong"))
        observe("wrong-secret.session", status)

    finish()
  '';
}
