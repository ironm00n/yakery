# NEU's calendar mirrored into the owner's Stalwart account, on cx33-1's bundles, against a stand-in
# for Entra and Graph at their real names: a sign-in through the callback, the subscription and
# its validation, a full listing, a notified change, a forged notification ignored, a missed
# notification made good by a fresh listing, and what the scheduler then counts as busy.
{ pkgs }:
let
  inherit (pkgs) lib;
  server = import ./scheduler-server.nix { inherit pkgs; };
  inherit (server) domain host;
  microsoft = [
    "graph.microsoft.com"
    "login.microsoftonline.com"
  ];
  fake = {
    port = 9440;
    log = "/var/lib/fake-graph/log.jsonl";
  };
in
pkgs.testers.runNixOSTest {
  name = "outlook-sync";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { config, ... }:
    {
      imports = [
        (server.nodeWith { tlsHosts = microsoft; })
        ../../bundles/outlook-sync.nix
      ];
      bundles.outlook-sync = {
        enable = true;
        owner = "owen@${domain}";
      };
      services.nginx.virtualHosts = lib.genAttrs microsoft (_: {
        onlySSL = true;
        # beside the bundles' vhosts, which take this address's connections otherwise
        listenAddresses = [ config.networking.primaryIPAddress ];
        locations."/".proxyPass = "http://127.0.0.1:${toString fake.port}";
      });
      systemd.services.fake-graph = {
        wantedBy = [ "multi-user.target" ];
        after = [ "outlook-sync-key.service" ];
        environment.SSL_CERT_FILE = "/etc/ssl/certs/ca-certificates.crt";
        serviceConfig.StateDirectory = "fake-graph";
        script = ''
          exec ${lib.getExe (pkgs.python3.withPackages (p: [ p.cryptography ]))} ${./fake-graph.py} \
            /var/lib/outlook-sync/certificate.pem ${fake.log}
        '';
      };
    };

  testScript = server.python + ''
    import urllib.parse

    start_all()
    wait_for_stalwart()
    server.wait_for_unit("fake-graph.service")
    server.wait_for_unit("outlook-sync.service")

    logins = sign_in(["owen", SCHEDULE])
    owen, owen_id = logins["owen"], account_ids["owen"]
    NEU = "/dav/cal/owen%40${domain}/neu/"
    FAKE = "http://127.0.0.1:${toString fake.port}"

    def fake_post(path, body=None):
        data = "" if body is None else "--data-binary " + shlex.quote(json.dumps(body))
        return json.loads(server.succeed(f"curl -sf -X POST {data} {shlex.quote(FAKE + path)}"))

    def fake_log(kind):
        lines = server.succeed("cat ${fake.log} 2>/dev/null || true").splitlines()
        return [e for e in map(json.loads, lines) if e["kind"] == kind]

    def occurrence(id, local, minutes=60, **extra):
        start = utc(local)
        return {"id": id, "subject": f"NEU {id}", "start": {"dateTime": start[:-1] + ".0000000", "timeZone": "UTC"},
                "end": {"dateTime": later(start, minutes)[:-1] + ".0000000", "timeZone": "UTC"},
                "isAllDay": False, "showAs": "busy", **extra}

    def mirrored():
        _, _, hrefs = propfind(owen, NEU)
        return sorted(h for h in hrefs if h != NEU)

    window = (utc(day(0) + " 00:00"), utc(day(7) + " 00:00"))

    def busy():
        got = jmap(logins[SCHEDULE], "Principal/getAvailability", {
            "accountId": account_ids[SCHEDULE], "id": owen_id, "utcStart": window[0], "utcEnd": window[1]})
        return [[p["utcStart"], p["utcEnd"]] for p in got.get("list", [])]

    def hours(*spec):
        return merged([at(local, minutes) for local, minutes in spec])

    with subtest("the plan makes neu in the owner's account, shared with the sync and the scheduler"):
        hrefs = observe("owner.home", propfind(owen, HOME)[2])
        expect("neu.in-owner-home", NEU in hrefs, hrefs)

    fake_post("/test/events", {"put": [
        occurrence("e1", f"{day(0)} 10:00"),
        occurrence("e2", f"{day(1)} 14:00", showAs="free"),
        occurrence("e3", f"{day(2)} 09:00", 90),
    ]})

    with subtest("sign-in: the command's link, Entra's redirect, the callback redeeming the code"):
        server.succeed("(outlook-sync login > /tmp/login.out 2>&1; echo $? > /tmp/login.exit) >/dev/null 2>&1 &")
        server.wait_until_succeeds("grep -q oauth2/v2.0/authorize /tmp/login.out", timeout=30)
        link = next(l for l in server.succeed("cat /tmp/login.out").split() if "/authorize?" in l)
        query = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(link).query))
        observe("login.query", {k: v for k, v in query.items() if k not in ("state", "code_challenge")})
        expect("login.pkce", query.get("code_challenge_method") == "S256" and query.get("redirect_uri") == "https://${host}/neu/callback", query)
        page = server.succeed(f"curl -sSL {shlex.quote(link)}")
        expect("login.callback", "Signed in" in page, page[-300:])
        server.wait_until_succeeds("test -s /tmp/login.exit", timeout=30)
        expect("login.command", server.succeed("cat /tmp/login.exit").strip() == "0", server.succeed("cat /tmp/login.out"))
        page = server.succeed(f"curl -s {shlex.quote(link)} -L")
        expect("login.once", "Signed in" not in page, page[-300:])
        tokens = observe("token.grants", fake_log("token"))
        expect("token.code-first", tokens and tokens[0]["grant"] == "authorization_code", tokens)

    with subtest("the subscription, validated on both URLs, and the first listing"):
        server.wait_until_succeeds(f"curl -sf {FAKE}/test/state | grep -q sub-1", timeout=60)
        observe("validations", fake_log("validate"))
        expect("validated.both", len([v for v in fake_log("validate") if v["status"] == 200 and v["echoed"]]) == 2, fake_log("validate"))
        sub = observe("subscribe", fake_log("subscribe"))
        expect("subscribe.mailbox-events", sub and sub[0]["resource"] == "/me/events", sub)
        retry(lambda _: len(mirrored()) >= 3, timeout_seconds=60)
        expect("listing.paged", len(fake_log("listing")) >= 2, fake_log("listing"))
        expect("mirror.three", len(observe("mirror.first", mirrored())) == 3, mirrored())
        got = observe("busy.first", busy())
        expect("busy.first", got == hours((f"{day(0)} 10:00", 60), (f"{day(2)} 09:00", 90)), got)

    with subtest("a notified change: one moved, one removed, one new"):
        fake_post("/test/events", {"put": [occurrence("e1", f"{day(0)} 15:00"), occurrence("e4", f"{day(3)} 11:00")], "remove": ["e3"]})
        fake_post("/test/notify")
        want = hours((f"{day(0)} 15:00", 60), (f"{day(3)} 11:00", 60))
        retry(lambda _: busy() == want, timeout_seconds=60)
        got = observe("busy.notified", busy())
        expect("busy.notified", got == want, got)
        expect("delta.incremental", any(d["since"] > 0 for d in fake_log("delta")), fake_log("delta"))
        expect("mirror.e3-gone", len(mirrored()) == 3, mirrored())

    with subtest("a lifecycle event with the wrong secret starts nothing; a true notification then does"):
        listings = len(fake_log("listing"))
        fake_post("/test/events", {"put": [occurrence("e5", f"{day(4)} 10:00")]})
        fake_post("/test/lifecycle?event=missed&clientState=forged")
        fake_post("/test/notify")
        retry(lambda _: len(mirrored()) == 4, timeout_seconds=60)
        expect("forged.no-listing", len(fake_log("listing")) == listings, fake_log("listing")[listings:])

    with subtest("missed notifications: a fresh listing makes them good"):
        fake_post("/test/events", {"put": [occurrence("e6", f"{day(4)} 16:00")]})
        fake_post("/test/lifecycle?event=missed")
        want = hours((f"{day(0)} 15:00", 60), (f"{day(3)} 11:00", 60), (f"{day(4)} 10:00", 60), (f"{day(4)} 16:00", 60))
        retry(lambda _: busy() == want, timeout_seconds=60)
        got = observe("busy.missed", busy())
        expect("busy.missed", got == want, got)
        expect("missed.listed-again", len(fake_log("listing")) > listings, fake_log("listing")[listings:])

    with subtest("a validation token outside a subscribe, and a stray callback, are refused"):
        status = server.succeed("curl -s -o /dev/null -w '%{http_code}' -X POST 'https://${host}/neu/notify?validationToken=x'")
        expect("validation.refused", status == "403", status)
        status = server.succeed("curl -s -o /dev/null -w '%{http_code}' 'https://${host}/neu/callback?state=x&code=y'")
        expect("callback.refused", status in ("403", "404"), status)

    with subtest("a restart keeps the subscription and the sign-in"):
        server.succeed("systemctl restart outlook-sync")
        server.wait_for_unit("outlook-sync.service")
        retry(lambda _: any(t["grant"] == "refresh_token" for t in fake_log("token")), timeout_seconds=60)
        state = json.loads(server.succeed(f"curl -sf {FAKE}/test/state"))
        expect("restart.one-subscription", state["subscriptions"] == 1, state)

    with subtest("Graph forgets the subscription: removed, subscribed again"):
        fake_post("/test/lifecycle?event=subscriptionRemoved")
        server.wait_until_succeeds(f"curl -sf {FAKE}/test/state | grep -q sub-2", timeout=60)
        observe("subscribe.again", fake_log("subscribe"))

    finish()
  '';
}
