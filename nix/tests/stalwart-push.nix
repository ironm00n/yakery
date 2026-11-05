# Push as RFC 8620 7.1 has it, run on cx33-1's Stalwart: a StateChange for every account the user
# can reach, a group's reached by share alone, over EventSource and Web Push; and none for a group
# the user has no share in.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain address;
  inherit (pkgs.callPackage ../../packages/stalwart { webuiOauthClientId = "test"; }) permissions;
  group = "calendars";
  calendar = "team";
  pushHost = "push.${domain}";
  pushLog = "/var/log/nginx/push.log";
  sink = {
    addr = "127.0.0.1";
    port = 8090;
  };
in
pkgs.testers.runNixOSTest {
  name = "stalwart-push";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { config, ... }:
    {
      imports = [
        (stalwart.node {
          users = [
            "owen"
            "outsider"
          ];
          tlsHosts = [ pushHost ];
        })
      ];
      bundles.stalwart.groups = [ group ];
      bundles.stalwart.programAccounts.sharee.permissions = with permissions; [
        authenticate
        jmapCalendarGet
        jmapCalendarCreate
        jmapCalendarEventGet
        jmapPushSubscriptionCreate
        jmapPushSubscriptionUpdate
      ];
      services.stalwart.calendars.${address group}.${calendar}.shares =
        lib.genAttrs
          (map address [
            "sharee"
            "owen"
          ])
          (_: [
            "mayReadItems"
            "mayWriteAll"
          ]);

      # the Web Push endpoint: nginx logs each body Stalwart posts, which it reads only to pass on
      services.nginx.commonHttpConfig = "log_format body escape=none '$request_body';";
      services.nginx.virtualHosts = {
        ${pushHost} = {
          onlySSL = true;
          listenAddresses = [ config.networking.primaryIPAddress ];
          locations."/" = {
            proxyPass = "http://${sink.addr}:${toString sink.port}";
            extraConfig = "access_log ${pushLog} body;";
          };
        };
        push-sink = {
          listen = [ sink ];
          locations."/".return = "200";
        };
      };
    };

  testScript = stalwart.python + ''
    import re

    GROUP = "${group}"
    USING = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:calendars"]

    start_all()
    wait_for_stalwart()

    logins = sign_in(["sharee", "owen", "outsider"])
    ids = observe("ids", account_ids)

    def jmap(name, method, args):
        status, payload = http(server, logins[name], "POST", "/jmap/", {"using": USING, "methodCalls": [[method, args, "0"]]})
        assert status == 200, payload
        got, result, _ = json.loads(payload)["methodResponses"][0]
        assert got == method, result
        return result

    def own_calendar(name):
        made = jmap(name, "Calendar/set", {"accountId": ids[name], "create": {"c": {"name": "Own"}}})
        assert "c" in (made.get("created") or {}), made
        return made["created"]["c"]["id"]

    def change(name, account, calendar):
        event = {"@type": "Event", "calendarIds": {calendar: True}, "title": f"by {name}", "start": "2026-12-01T10:00:00", "duration": "PT1H"}
        made = observe(f"change.{name}.{account}", jmap(name, "CalendarEvent/set", {"accountId": ids[account], "create": {"e": event}}))
        assert "e" in (made.get("created") or {}), made

    def arrives(path, account):
        try:
            server.wait_until_succeeds(f"grep -qF {shlex.quote(json.dumps(ids[account]) + ':')} {path}", timeout=60)
            return True
        except Exception:
            return False

    def pushed():
        return [json.loads(line) for line in server.succeed("cat ${pushLog}").splitlines()]

    with subtest("what each session lists"):
        for n in logins:
            observe(f"session.{n}", sorted(a["name"] for a in session(server, logins[n])[1]["accounts"].values()))

    team = next(c["id"] for c in jmap("owen", "Calendar/get", {"accountId": ids[GROUP]})["list"] if c["name"] == "${calendar}")
    own = {n: own_calendar(n) for n in ["owen", "outsider"]}

    # the push manager registers a verified subscription's accounts together and on its own time, so
    # a change in the sharee's own account reaching the endpoint shows the group's is registered too
    with subtest("the sharee's Web Push subscription, verified"):
        made = observe("push.create", jmap("sharee", "PushSubscription/set", {"create": {"p": {"deviceClientId": "test", "url": "https://${pushHost}/sharee"}}}))
        push_id = made["created"]["p"]["id"]
        server.wait_until_succeeds("grep -q PushVerification ${pushLog}", timeout=60)
        code = next(p["verificationCode"] for p in pushed() if p["@type"] == "PushVerification")
        made = observe("push.verify", jmap("sharee", "PushSubscription/set", {"update": {push_id: {"verificationCode": code}}}))
        assert push_id in (made.get("updated") or {}), made
        own_calendar("sharee")
        expect("push.sharee.own", arrives("${pushLog}", "sharee"), pushed())

    def listen(name):
        auth = shlex.quote("Authorization: " + logins[name])
        url = shlex.quote(origin(server) + "/jmap/eventsource/?types=*&closeafter=no&ping=30")
        server.succeed(f"systemd-run --unit=es-{name} -p StandardOutput=file:/tmp/es-{name}.txt ${lib.getExe pkgs.curl} -sSiN -H {auth} {url}")

    def stream(name):
        text = observe(f"eventsource.{name}", server.succeed(f"cat /tmp/es-{name}.txt"))
        return [json.loads(data)["changed"] for data in re.findall(r"^event: state\ndata: (.*)$", text, re.M)]

    with subtest("a change in the group's calendar, then one in each user's own"):
        for n in logins:
            listen(n)
        # Stalwart subscribes before it answers, and its headers go out with the first ping, 30 s in
        for n in logins:
            server.wait_until_succeeds(f"grep -q '^HTTP/1.1 200' /tmp/es-{n}.txt", timeout=60)
        change("owen", GROUP, team)
        change("owen", "owen", own["owen"])
        change("outsider", "outsider", own["outsider"])

    def reached(changes):
        return sorted({account for changed in changes for account in changed})

    with subtest("each stream has the accounts it can reach, and only those"):
        for n, account in [("sharee", GROUP), ("owen", "owen"), ("owen", GROUP), ("outsider", "outsider")]:
            arrives(f"/tmp/es-{n}.txt", account)
        arrives("${pushLog}", GROUP)
        got = {n: reached(stream(n)) for n in logins}
        got["push.sharee"] = reached(p["changed"] for p in observe("push", pushed()) if p["@type"] == "StateChange")
        observe("reached", got)
        expect("sharee.group", ids[GROUP] in got["sharee"], got["sharee"])
        expect("owen.own", ids["owen"] in got["owen"], got["owen"])
        expect("owen.group", ids[GROUP] in got["owen"], got["owen"])
        # changes reach a stream in the order they are made, so with the outsider's own change in, any for the group would be
        expect("outsider.own", ids["outsider"] in got["outsider"], got["outsider"])
        expect("outsider.no-group", ids[GROUP] not in got["outsider"], got["outsider"])
        expect("push.sharee.group", ids[GROUP] in got["push.sharee"], got["push.sharee"])

    finish()
  '';
}
