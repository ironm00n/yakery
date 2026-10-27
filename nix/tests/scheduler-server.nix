# cx33-1's Stalwart for the scheduler's VM tests: the owner, one program account per counted
# calendar, schedule@ for Bookings, and an SMTP sink standing in for every other server; and
# Python helpers for JMAP and the owner's calendars.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart.my-lib.stalwart) update upsertOn1;
  sink = {
    port = 2525;
    dir = "/var/lib/sink";
  };
in
rec {
  inherit (stalwart) domain client;
  calendarAccounts = [
    "work"
    "personal"
    "other"
    "outlook"
  ];
  programAccounts =
    let
      scheduler = import ../../crates/scheduler/accounts.nix {
        inherit lib;
        calendars = calendarAccounts;
      };
    in
    scheduler.core
    // {
      ${scheduler.feed.name} = scheduler.feed.permissions;
    }
    // (with (pkgs.callPackage ../../packages/stalwart { webuiOauthClientId = "test"; }).permissions; {
      # Q1's other answer: creates, and no invitations
      schedule-nosend = [
        authenticate
        jmapCalendarGet
        jmapCalendarEventCreate
      ];
      # every read permission, so what confines it is the free/busy share alone
      schedule-probe = [
        authenticate
        jmapPrincipalGetAvailability
        jmapCalendarGet
        jmapCalendarEventGet
        jmapCalendarEventQuery
        jmapCalendarEventChanges
        davCalGet
        davCalPropFind
      ];
    });

  node = {
    imports = [ (stalwart.node { users = [ "owen" ]; }) ];
    bundles.stalwart.programAccounts = lib.mapAttrs (_: permissions: {
      inherit permissions;
    }) programAccounts;
    services.stalwart.plan.sequence = lib.mkAfter [
      (upsertOn1 "MtaRoute" "sink" [ "name" ] {
        "@type" = "Relay";
        name = "sink";
        address = "127.0.0.1";
        inherit (sink) port;
        protocol = "smtp";
      })
      (update "MtaOutboundStrategy" {
        route = {
          match = [
            {
              "if" = "is_local_domain(rcpt_domain)";
              "then" = "'local'";
            }
          ];
          "else" = "'sink'";
        };
      })
    ];
    systemd.services.sink = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig.StateDirectory = baseNameOf sink.dir;
      script = "${lib.getExe' pkgs.python3Packages.aiosmtpd "aiosmtpd"} -n -l 127.0.0.1:${toString sink.port} -c aiosmtpd.handlers.Mailbox ${sink.dir}/mail";
    };
  };

  python = stalwart.python + ''
    import datetime
    import email
    import email.policy
    import re

    NY = "America/New_York"
    observed = {}
    failures = []

    def observe(key, value):
        observed[key] = value
        print("OBSERVE " + json.dumps({key: value}, sort_keys=True))
        return value

    def expect(key, ok, value=None):
        print(("EXPECT ok " if ok else "EXPECT FAIL ") + key + ("" if value is None else " " + json.dumps(value, sort_keys=True)))
        if not ok:
            failures.append(key)

    def http(node, auth, method, path, body=None, ctype="application/json", headers=()):
        args = [f"-X {method}", "-H " + shlex.quote("Authorization: " + auth)]
        args += ["-H " + shlex.quote(h) for h in headers]
        if body is not None:
            data = body if isinstance(body, str) else json.dumps(body)
            args += ["-H " + shlex.quote("Content-Type: " + ctype), "--data-binary " + shlex.quote(data)]
        out = node.succeed(f"curl -s -w '\\n%{{http_code}}' {' '.join(args)} {origin(node)}{path}")
        payload, status = out.rsplit("\n", 1)
        return int(status), payload

    CAL = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:calendars", "urn:ietf:params:jmap:principals"]
    MAIL = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"]

    def jmap(auth, method, args, using=CAL):
        status, payload = http(server, auth, "POST", "/jmap/", {"using": using, "methodCalls": [[method, args, "0"]]})
        if status != 200:
            return {"http": status, "body": payload[:300]}
        name, result, _ = json.loads(payload)["methodResponses"][0]
        return result if name == method else {"error": result}

    def forbidden(result):
        return result.get("error", {}).get("type") == "forbidden"

    def utc(local, tz=NY):
        spec = 'TZ="' + tz + '" ' + local
        return server.succeed("date -u -d " + shlex.quote(spec) + " +%Y-%m-%dT%H:%M:%SZ").strip()

    def parse(t):
        return datetime.datetime.strptime(t, "%Y-%m-%dT%H:%M:%SZ")

    def later(t, minutes):
        return (parse(t) + datetime.timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")

    def merged(periods):
        out = []
        for s, e in sorted(periods):
            if out and s <= out[-1][1]:
                out[-1] = (out[-1][0], max(out[-1][1], e))
            else:
                out.append((s, e))
        return [list(p) for p in out]

    def sink_mail():
        return server.succeed("ls ${sink.dir}/mail/new").split()

    def sink_message(name):
        return email.message_from_string(server.succeed(f"cat ${sink.dir}/mail/new/{name}"), policy=email.policy.default)

    def sign_in(names):
        return {n: basic(f"{n}@${domain}", app_password(n)) for n in names}

    today = datetime.date.today()
    monday = today + datetime.timedelta(days=7 - today.weekday())
    if (monday - today).days < 3:
        monday += datetime.timedelta(days=7)

    def day(n):
        return (monday + datetime.timedelta(days=n)).isoformat()

    def ev(calendar, title, local, minutes=60, tz=NY, **extra):
        e = {"@type": "Event", "calendarIds": {calendar: True}, "title": title,
             "start": local, "duration": f"PT{minutes}M", **extra}
        if tz:
            e["timeZone"] = tz
        return e

    def create(auth, events, **args):
        return jmap(auth, "CalendarEvent/set", {"accountId": owen_id, "create": events, **args})

    def calendars(spec):
        made = jmap(owen, "Calendar/set", {"accountId": owen_id, "create": spec})
        assert not made.get("notCreated"), made
        return {k: v["id"] for k, v in made["created"].items()}

    def share(calendar, account, rights):
        made = jmap(owen, "Calendar/set", {"accountId": owen_id, "update": {calendar: {"shareWith/" + account_ids[account]: rights}}})
        assert not made.get("notUpdated"), made

    def propfind(auth, path):
        status, body = http(server, auth, "PROPFIND", path, "<propfind xmlns='DAV:'><prop><displayname/></prop></propfind>", "application/xml", ["Depth: 1"])
        return status, body, re.findall(r"<D:href>([^<]+)</D:href>", body)

    HOME = "/dav/cal/owen%40${domain}/"

    def calendar_href(name):
        _, body, hrefs = propfind(owen, HOME)
        return next(h for h in hrefs if f"<D:displayname>{name}</D:displayname>" in body.split(h, 1)[1].split("</D:response>", 1)[0])

    FREE_BUSY = {"mayReadFreeBusy": True}
    BOOKINGS = {"mayReadFreeBusy": True, "mayReadItems": True, "mayWriteAll": True}

    def finish():
        server.succeed("echo " + shlex.quote(json.dumps(observed, sort_keys=True, indent=1)) + " > /tmp/observed.json")
        server.copy_from_machine("/tmp/observed.json", "")
        assert not failures, failures
  '';
}
