# agent-mail against ZITADEL and cx33-1's Stalwart, each on its own server as in production:
# host1 and host2 share the login agent@, and owner@ on host1 mails it.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) my-lib domain;
  inherit (my-lib.ironmoon) auth mail;
  logins = {
    agent = "${mail.agents.sharedLocalPart}@${domain}";
    owner = "owner@${domain}";
  };
  password = "Agent-mail-test-1";
  # where the redirect tests send agent-mail; nothing may reach it
  evil = "evil.test";
  zitadel = import ./zitadel-server.nix { inherit pkgs my-lib; };

  tls = pkgs.runCommand "agent-mail-test-tls" { nativeBuildInputs = [ pkgs.minica ]; } ''
    mkdir $out && cd $out
    minica --domains ${auth.host}
  '';

  # agent-mail drops SSL_CERT_FILE and the like, so the CA has to be in the system store
  network =
    { nodes, ... }:
    {
      security.pki.certificateFiles = [ "${tls}/minica.pem" ];
      networking.hosts.${nodes.auth.networking.primaryIPAddress} = [ auth.host ];
    };

  zitadelServer = {
    imports = [
      zitadel.node
      network
    ];
    virtualisation = {
      memorySize = 4096;
      cores = 2;
    };
    services.zitadel.settings.DefaultInstance.Features.LoginV2.Required = false;
    services.nginx.virtualHosts.${auth.host} = {
      enableACME = lib.mkForce false;
      sslCertificate = "${tls}/${auth.host}/cert.pem";
      sslCertificateKey = "${tls}/${auth.host}/key.pem";
    };
  };

  # ZITADEL picks every client id itself, so a seeded instance is dumped once and restored for each run
  seed = pkgs.testers.runNixOSTest {
    name = "agent-mail-zitadel-seed";

    defaults.virtualisation.graphics = false;

    nodes.auth = zitadelServer;

    testScript =
      { nodes, ... }:
      zitadel.python
      + ''
        zitadel = auth
        https = "curl -fsS https://${auth.host}"

        auth.wait_for_unit("nginx.service")
        auth.wait_until_succeeds(f"{https}/debug/ready", timeout=300)
        pat = admin_pat()
        project = api("POST", "/management/v1/projects", pat, {"name": "ironmoon"})["id"]

        def app(name, **settings):
            return api("POST", f"/management/v1/projects/{project}/apps/oidc", pat, {
                "name": name,
                "responseTypes": ["OIDC_RESPONSE_TYPE_CODE"],
                "authMethodType": "OIDC_AUTH_METHOD_TYPE_NONE",
                "accessTokenType": "OIDC_TOKEN_TYPE_JWT",
                **settings,
            })["clientId"]

        def human(login):
            return api("POST", "/management/v1/users/human/_import", pat, {
                "userName": login,
                "profile": {"firstName": login.split("@")[0], "lastName": "test"},
                "email": {"email": login, "isEmailVerified": True},
                "password": "${password}",
                "passwordChangeRequired": False,
            })["userId"]

        ids = {
            "projectId": project,
            "webuiClientId": app(
                "stalwart-webui",
                appType="OIDC_APP_TYPE_USER_AGENT",
                grantTypes=["OIDC_GRANT_TYPE_AUTHORIZATION_CODE"],
                redirectUris=["https://${mail.host}/"],
            ),
            "agentMailClientId": app(
                "agent-mail",
                appType="OIDC_APP_TYPE_NATIVE",
                grantTypes=["OIDC_GRANT_TYPE_DEVICE_CODE", "OIDC_GRANT_TYPE_REFRESH_TOKEN"],
            ),
            "userIds": {login: human(login) for login in ${builtins.toJSON (lib.attrValues logins)}},
            "adminPat": pat,
        }
        auth.succeed(
            "systemctl stop zitadel",
            "${nodes.auth.bundles.backup.services.zitadel.dumpCommand} > /tmp/zitadel.dump",
            f"echo '{json.dumps(ids)}' > /tmp/ids.json",
        )
        auth.copy_from_machine("/tmp/zitadel.dump")
        auth.copy_from_machine("/tmp/ids.json")
      '';
  };

  agent-mail =
    (pkgs.callPackage ../../crates/agent-mail/package.nix {
      deployment = my-lib.ironmoon;
      correspondents = [ ];
      correspondentsFor = { };
    }).overrideAttrs
      (old: {
        preBuild = (old.preBuild or "") + ''
          export AGENT_MAIL_CLIENT_ID=$(${lib.getExe pkgs.jq} -r .agentMailClientId ${seed}/ids.json)
        '';
      });

  client = users: {
    imports = [
      network
      stalwart.client
    ];
    environment.systemPackages = [
      agent-mail
      pkgs.curl
    ];
    users.users = lib.genAttrs users (_: {
      isNormalUser = true;
    });
    systemd.tmpfiles.settings.agent-mail = lib.mergeAttrsList (
      map (
        name:
        let
          home = "/home/${name}";
          own = mode: {
            user = name;
            group = "users";
            inherit mode;
          };
          config = pkgs.writeText "agent-mail-${name}.json" (
            builtins.toJSON {
              login = logins.${name};
              password-file = "~/.config/agent-mail/password";
            }
          );
        in
        {
          "${home}/.config".d = own "0700";
          "${home}/.config/agent-mail".d = own "0700";
          "${home}/.config/agent-mail/config.json"."L+".argument = "${config}";
          "${home}/.config/agent-mail/password".f = own "0600" // {
            argument = password;
          };
        }
      ) users
    );
  };
in
pkgs.testers.runNixOSTest {
  name = "agent-mail";

  defaults.virtualisation.graphics = false;

  nodes = {
    auth =
      { config, ... }:
      let
        postgres = config.services.postgresql.package;
      in
      {
        imports = [ zitadelServer ];
        systemd.services.zitadel-restore = {
          requiredBy = [ "zitadel.service" ];
          before = [ "zitadel.service" ];
          requires = [ "postgresql.target" ];
          after = [ "postgresql.target" ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            User = "zitadel";
          };
          # into the empty database only, which a reboot finds restored already
          script = ''
            if [ "$(${postgres}/bin/psql --host=/run/postgresql -tAc "select to_regnamespace('eventstore') is null" zitadel)" = t ]; then
              ${postgres}/bin/pg_restore --no-owner --host=/run/postgresql --dbname=zitadel ${seed}/zitadel.dump
            fi
          '';
        };
      };

    server =
      { config, ... }:
      {
        imports = [
          (stalwart.node { tlsHosts = [ evil ]; })
          network
        ];

        # Stalwart fetches ZITADEL's discovery document and keys only as it starts
        systemd.services.zitadel-ready = {
          requiredBy = [ "stalwart.service" ];
          before = [ "stalwart.service" ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
          };
          script = "until ${lib.getExe pkgs.curl} -fsS -o /dev/null https://${auth.host}/oauth/v2/keys; do sleep 1; done";
        };

        # cx33-1's blob store is a round trip of about 25 ms away; Stalwart's S3 calls are all that
        # reach nginx's port 443 over loopback here, and each waits that long on the way out
        boot.kernelModules = [
          "sch_prio"
          "sch_netem"
          "cls_u32"
        ];
        systemd.services.s3-latency = {
          requiredBy = [ "stalwart.service" ];
          before = [ "stalwart.service" ];
          path = [ pkgs.iproute2 ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
          };
          script = ''
            tc qdisc add dev lo root handle 1: prio bands 4
            tc qdisc add dev lo parent 1:4 handle 40: netem delay 25ms
            tc filter add dev lo parent 1: protocol ip u32 match ip dport 443 0xffff flowid 1:4
          '';
        };

        # Stalwart trusts a bearer it has checked for this long, whatever its exp, so the 75 s tokens of the
        # expiry test need a shorter hold than the hour it defaults to (http/src/auth/authenticate.rs:31-48).
        # An update must name the provider's own keys, which Stalwart's sign-in, unused here, would sign with.
        services.stalwart.plan.sequence = lib.mkAfter [
          (my-lib.stalwart.update "OidcProvider" {
            accessTokenExpiry = 30 * 1000;
            encryptionKey = {
              "@type" = "Value";
              secret = "agent-mail-test-encryption";
            };
            signatureAlgorithm = "hs256";
            signatureKey = {
              "@type" = "Text";
              secret = "agent-mail-test-signature";
            };
          })
        ];

        # switched on by a file, for the redirect tests
        bundles.reverse-proxy.hosts.${mail.host}.locations = {
          "= /.well-known/jmap".extraConfig = ''
            if (-f /run/redirect/off) { return 302 https://${evil}/collect; }
            if (-f /run/redirect/loop) { return 302 /.well-known/jmap; }
          '';
          "= /jmap/".extraConfig = "if (-f /run/redirect/api) { return 307 /jmap/; }";
        };
        services.nginx.virtualHosts.${evil} = {
          onlySSL = true;
          listenAddresses = [ config.networking.primaryIPAddress ];
          extraConfig = "access_log /var/log/nginx/evil.log;";
          locations."/".return = "200";
        };
      };
    host1 = client [
      "agent"
      "owner"
    ];
    host2 = client [ "agent" ];
  };

  testScript = stalwart.python + ''
    import collections
    import re
    import time

    ids = json.load(open("${seed}/ids.json"))
    agent, owner = "runuser -u agent -- agent-mail", "runuser -u owner -- agent-mail"
    topic = "agent+smoke@${domain}"

    # the email id agent-mail prints after a prefix: "sent" for send, "===" for each mail of wait
    def email_id(prefix, output):
        found = re.match(rf"{prefix} (\S+) ", output)
        assert found, output
        return found.group(1)

    start_all()
    wait_for_stalwart()

    with subtest("the build compiled in the seeded client id"):
        host1.succeed(f"grep -qa {ids['agentMailClientId']} ${lib.getExe agent-mail}")
        host1.fail("grep -qa ${auth.agentMailClientId} ${lib.getExe agent-mail}")

    with subtest("Stalwart takes the tokens only once its directory requires the seeded webui client"):
        host1.succeed(f"{owner} login")
        status, refused = host1.execute(f"{owner} inbox 2>&1")
        assert status == 1 and "stalwart: /jmap/session: HTTP 401 Unauthorized" in refused, refused
        directory = call(admin, admin_id, "x:Directory/query", {})["ids"][0]
        updated = call(admin, admin_id, "x:Directory/set", {"update": {directory: {"requireAudience": ids["webuiClientId"]}}})
        assert directory in updated.get("updated", {}), updated
        # a ReloadSettings action would also rebuild SpamPyzor, whose server can't be resolved here, and so fail
        server.succeed("systemctl restart stalwart")
        wait_for_stalwart()

    # Stalwart makes the account of an OIDC login at its first JMAP session, so each opens one before mail is sent to it
    with subtest("login signs in through the device flow, and Stalwart takes the token"):
        for node, user in [(host1, agent), (host2, agent), (host1, owner)]:
            node.succeed(f"{user} login", f"{user} inbox")

    with subtest("wait prints owner's mail to the topic and sends a read receipt"):
        sent = host1.succeed(f"echo '**smoke** test' | {owner} send --to {topic} --subject smoke")
        sent_id = email_id("sent", sent)
        shown = host1.succeed(f"{agent} wait --topic smoke --timeout 2m")
        assert "**smoke** test" in shown, shown
        smoke_id = email_id("===", shown)
        host1.wait_until_succeeds(f"{owner} inbox | grep -F '{topic} -> ${logins.owner}'", timeout=60)

    with subtest("the reply goes out from the topic, and owner's thread shows mail, reply and receipt"):
        host1.succeed(f"echo reply | {agent} send --reply {smoke_id} --to ${logins.owner}")
        host1.wait_until_succeeds(f"{owner} thread {sent_id} | grep -qF '{topic} -> ${logins.owner}'", timeout=60)
        frames = [line for line in host1.succeed(f"{owner} thread {sent_id}").splitlines() if line.startswith("=== ")]
        assert len(frames) == 3, frames
        assert f"${logins.owner} -> {topic}  smoke" in frames[0], frames
        assert frames[1].startswith(f"=== read by {topic} at "), frames
        assert f"{topic} -> ${logins.owner}  " in frames[2], frames

    with subtest("done keeps the mail from wait, which times out with 2, and finds no email under an id Stalwart can't read"):
        host1.succeed(f"{agent} done {smoke_id}")
        status, _ = host1.execute(f"{agent} wait --topic smoke --timeout 20s")
        assert status == 2, status
        status, refused = host1.execute(f"{agent} done x_y 2>&1")
        assert status == 1 and "no email x_y" in refused, refused

    with subtest("attachments land under the passwd home whatever HOME says"):
        host1.succeed("echo attached > /tmp/note.txt")
        host1.succeed(f"echo see attached | {owner} send --to {topic} --subject files --attach /tmp/note.txt")
        files_id = email_id("===", host1.succeed(f"{agent} wait --topic smoke --timeout 2m"))
        saved = host1.succeed(
            "runuser -u agent -- env HOME=/tmp/elsewhere XDG_CONFIG_HOME=/home/agent/.config XDG_STATE_HOME=/home/agent/.local/state"
            f" agent-mail attachments {files_id}"
        ).split()
        assert saved == [f"/home/agent/.cache/agent-mail/attachments/{files_id}/note.txt"], saved
        host1.succeed(f"cmp /tmp/note.txt {saved[0]}", "test ! -e /tmp/elsewhere")

    with subtest("attachments also saves the media both bodies show inline and the images the HTML embeds"):
        host1.succeed(
            "printf '%s\\r\\n' 'Content-Type: multipart/mixed; boundary=m' ''' '--m'"
            " 'Content-Type: text/html; charset=utf-8' ''' '<p>see</p><img src=\"data:image/png;base64,iVBORw0KGgo=\">'"
            " '--m' 'Content-Type: image/gif' 'Content-Disposition: inline; filename=dot.gif'"
            " 'Content-Transfer-Encoding: base64' ''' 'R0lGODlhAQABAAAAACw=' '--m--' > /tmp/inline.eml"
        )
        host1.succeed(f"{owner} send --to {topic} --subject inline --mime /tmp/inline.eml")
        inline_id = email_id("===", host1.succeed(f"{agent} wait --topic smoke --timeout 2m"))
        folder = f"/home/agent/.cache/agent-mail/attachments/{inline_id}"
        saved = host1.succeed(f"{agent} attachments {inline_id}").split()
        assert saved == [f"{folder}/dot.gif", f"{folder}/embedded-2.png"], saved
        host1.succeed(
            f"echo R0lGODlhAQABAAAAACw= | base64 -d | cmp - {folder}/dot.gif",
            f"echo iVBORw0KGgo= | base64 -d | cmp - {folder}/embedded-2.png",
        )

    with subtest("password rotate changes the password, and a fresh sign-in takes the new one"):
        host1.succeed(f"{owner} password rotate")
        host1.fail("grep -qxF '${password}' /home/owner/.config/agent-mail/password")
        host1.succeed("rm /home/owner/.local/state/agent-mail/tokens.json", f"{owner} login")

    with subtest("a redirect is followed only on the origin, never off it, and never for an API call"):
        server.succeed("mkdir -p /run/redirect")
        for switch, refused in [
            ("off", "refusing https://${evil}/collect: it is not on https://${mail.host}"),
            ("loop", "/.well-known/jmap: more than 10 redirects"),
            ("api", "stalwart: /jmap/: HTTP 307 Temporary Redirect"),
        ]:
            server.succeed(f"touch /run/redirect/{switch}")
            status, out = host1.execute(f"{agent} inbox 2>&1")
            server.succeed(f"rm /run/redirect/{switch}")
            assert status == 1 and refused in out, out
        server.succeed("test ! -s /var/log/nginx/evil.log")
        host1.succeed(f"{agent} inbox")

    bearer = lambda node: json.loads(node.succeed("cat /home/agent/.local/state/agent-mail/tokens.json"))["access"]["token"]
    using = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"]
    json_type = "-H 'Content-Type: application/json'"

    def jmap(token, method, args):
        body = shlex.quote(json.dumps({"using": using, "methodCalls": [[method, args, "c"]]}))
        out = host1.succeed(f"curl -fsS -H 'Authorization: Bearer {token}' {json_type} --data {body} https://${mail.host}/jmap/")
        return json.loads(out)["methodResponses"][0][1]

    with subtest("of two hosts' marks racing for one mail Stalwart writes one, the only one a claim takes"):
        tokens = [bearer(host1), bearer(host2)]
        discovered = json.loads(host1.succeed(f"curl -fsSL -H 'Authorization: Bearer {tokens[0]}' https://${mail.host}/.well-known/jmap"))
        account = discovered["primaryAccounts"]["urn:ietf:params:jmap:mail"]
        inbox = next(m["id"] for m in jmap(tokens[0], "Mailbox/get", {"accountId": account})["list"] if m["role"] == "inbox")
        message = shlex.quote("From: ${logins.owner}\r\nTo: agent+probe@${domain}\r\nSubject: probe\r\n\r\nprobe\r\n")
        upload = f"curl -fsS -H 'Authorization: Bearer {tokens[0]}' --data-binary @- https://${mail.host}/jmap/upload/{account}/"
        blob = json.loads(host1.succeed(f"printf %s {message} | {upload}"))["blobId"]

        def race(if_in_state):
            imported = {"r": {"blobId": blob, "mailboxIds": {inbox: True}}}
            email = jmap(tokens[0], "Email/import", {"accountId": account, "emails": imported})["created"]["r"]["id"]
            args = {"accountId": account, "update": {email: {"keywords/$seen": True}}}
            if if_in_state:
                args["ifInState"] = jmap(tokens[0], "Email/get", {"accountId": account, "ids": []})["state"]
            body = shlex.quote(json.dumps({"using": using, "methodCalls": [["Email/set", args, "c"]]}))
            post = f"curl -sS {json_type} --data @/tmp/race.json https://${mail.host}/jmap/ -H"
            out = host1.succeed(
                f"printf %s {body} > /tmp/race.json && ({post} 'Authorization: Bearer {tokens[0]}' > /tmp/a.json &"
                f" {post} 'Authorization: Bearer {tokens[1]}' > /tmp/b.json & wait) && cat /tmp/a.json && echo && cat /tmp/b.json"
            )

            def outcome(reply):
                name, result, _ = json.loads(reply)["methodResponses"][0]
                if name == "error":
                    return result["type"]
                if email in (result.get("notUpdated") or {}):
                    return result["notUpdated"][email]["type"]
                return "wrote" if result["newState"] != result["oldState"] else "unchanged"

            return tuple(sorted(outcome(reply) for reply in out.strip().splitlines()))

        rounds = 30
        old, new = (collections.Counter(race(if_in_state) for _ in range(rounds)) for if_in_state in [True, False])
        print(f"two marks of one mail, with ifInState: {dict(old)}; one per mail, without it: {dict(new)}")
        assert all(pair.count("wrote") == 1 for pair in list(old) + list(new)), (old, new)
        # the old claim took any answer but stateMismatch as its own, forbidden through its lost-reply path
        twice = sum(n for pair, n in old.items() if "stateMismatch" not in pair)
        print(f"the claim before the rewrite would have delivered {twice} of {rounds} mails twice")

    # how far a node's clock is ahead of the driver's, give or take half a round trip
    def offset(node):
        sent = time.time()
        return float(node.succeed("date +%s.%N")) - (sent + time.time()) / 2

    with subtest("a wait scans an inbox of production's size, each message a round trip away in S3, within a request's bound"):
        def stored(n):
            raw = (
                f"From: ${logins.owner}\r\nTo: agent+other{n % 20}@${domain}\r\nSubject: open {n}\r\nMIME-Version: 1.0\r\n"
                "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n"
                f"open {n}\r\n--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=n.txt\r\n\r\n{n}\r\n--b--\r\n"
            )
            return json.loads(host1.succeed(f"printf %s {shlex.quote(raw)} | {upload}"))["blobId"]

        stored_ids = [stored(n) for n in range(600)]
        for at in range(0, len(stored_ids), 100):
            emails = {str(i): {"blobId": b, "mailboxIds": {inbox: True}} for i, b in enumerate(stored_ids[at:at + 100])}
            created = jmap(tokens[0], "Email/import", {"accountId": account, "emails": emails})["created"]
            assert len(created) == len(emails), created
        # so every message comes from the bucket, as months-old mail does
        server.succeed("systemctl restart stalwart")
        wait_for_stalwart()
        status, out = host1.execute(f"{agent} wait --topic quiet --timeout 40s 2>&1")
        assert status == 2 and "timed out" not in out, out

    with subtest("waits on two hosts claim each mail of a topic once"):
        both = "agent+both@${domain}"
        offsets = [offset(node) for node in [host1, host2]]
        print(f"clocks ahead of the driver's by {offsets} s")
        for round in range(3):
            for n in range(4):
                host1.succeed(f"echo {n} | {owner} send --to {both} --subject both-{round}-{n}")
            host1.wait_until_succeeds(f"test $({agent} inbox --topic both --limit 100 | grep -c ' both-{round}-') = 4", timeout=60)
            start = time.time() + 3
            for node, ahead in zip([host1, host2], offsets):
                at = int((start + ahead) * 1e9)
                node.succeed(
                    f"systemd-run --uid=agent -E PATH=/run/current-system/sw/bin --unit=both-{round} -p StandardOutput=file:/tmp/both-{round}.out"
                    f" sh -c 'while [ $(date +%s%N) -lt {at} ]; do :; done; exec agent-mail wait --topic both --timeout 15s'"
                )
            claimed = []
            for node in [host1, host2]:
                node.wait_until_fails(f"systemctl is-active both-{round}", timeout=60)
                claimed.append(re.findall(r"^=== (\S+) ", node.succeed(f"cat /tmp/both-{round}.out"), re.M))
            print(f"round {round}: host1 claimed {claimed[0]}, host2 {claimed[1]}")
            assert not set(claimed[0]) & set(claimed[1]) and len(claimed[0]) + len(claimed[1]) == 4, claimed

    with subtest("a wait signs in again when its token expires, and Stalwart's pings wake nothing meanwhile"):
        def access_tokens_last(lifetime):
            lifetimes = {"idTokenLifetime": "43200s", "refreshTokenIdleExpiration": "2592000s", "refreshTokenExpiration": "7776000s"}
            body = shlex.quote(json.dumps({"accessTokenLifetime": lifetime, **lifetimes}))
            server.succeed(
                f"curl -fsS -X PUT https://${auth.host}/admin/v1/settings/oidc {json_type}"
                f" -H 'Authorization: Bearer {ids['adminPat']}' --data {body}"
            )

        access_tokens_last("75s")
        # Stalwart pings every 30 s at the most often, as agent-mail asks
        pings = f"curl -sN -H 'Authorization: Bearer {bearer(host2)}' 'https://${mail.host}/jmap/eventsource/?types=Email&closeafter=no&ping=30'"
        # read through a process substitution, since under pipefail the killed curl would fail a pipe
        host2.succeed(f"grep -m1 -q '^event: ping' <(timeout 40 {pings})")
        host2.succeed("rm /home/agent/.local/state/agent-mail/tokens.json", f"{agent} login")
        host2.succeed(
            "systemd-run --uid=agent -E PATH=/run/current-system/sw/bin --unit=expiring -p StandardOutput=file:/tmp/expiring.out"
            " -p StandardError=file:/tmp/expiring.err agent-mail wait --topic expiring --timeout 8m"
        )
        host2.wait_until_succeeds("grep -q 'waiting for' /tmp/expiring.err", timeout=30)
        time.sleep(10)
        address = host2.succeed("ip -4 -o addr show eth1 | awk '{print $4}' | cut -d/ -f1").strip()
        before = int(server.succeed("wc -l < /var/log/nginx/access.log"))
        time.sleep(120)
        quiet = server.succeed(f"tail -n +{before + 1} /var/log/nginx/access.log | grep -c '^{address} .*POST /jmap/' || true")
        assert quiet.strip() == "0", quiet
        host2.wait_until_succeeds("grep -q 'the access token expired; signing in again in 5s' /tmp/expiring.err", timeout=120)
        host2.wait_until_succeeds("grep -q 'signed in again; waiting' /tmp/expiring.err", timeout=60)
        host1.succeed(f"echo after the refresh | {owner} send --to agent+expiring@${domain} --subject expiring")
        host2.wait_until_succeeds("grep -q 'after the refresh' /tmp/expiring.out", timeout=120)
        access_tokens_last("43200s")

    # a wait as agent on host1, its exit status written beside its output once it ends
    def waiting(unit, timeout):
        host1.succeed(
            f"systemd-run --uid=agent -E PATH=/run/current-system/sw/bin --unit={unit} -p StandardOutput=file:/tmp/{unit}.out"
            f" -p StandardError=file:/tmp/{unit}.err sh -c 'agent-mail wait --topic {unit} --timeout {timeout}; echo $? > /tmp/{unit}.status'"
        )
        host1.wait_until_succeeds(f"grep -q 'waiting for' /tmp/{unit}.err", timeout=30)

    with subtest("a wait rides out Stalwart restarting for a few seconds"):
        waiting("restarted", "10m")
        # most of a recheck period after its last scan, so the restart comes as long after one as it can
        time.sleep(140)
        server.succeed("systemctl restart stalwart")
        wait_for_stalwart()
        host1.succeed(f"echo after the restart | {owner} send --to agent+restarted@${domain} --subject restarted")
        host1.wait_until_succeeds("test -e /tmp/restarted.status", timeout=180)
        err = host1.succeed("cat /tmp/restarted.err")
        assert host1.succeed("cat /tmp/restarted.status").strip() == "0" and "no scan finished" not in err, err
        host1.succeed("grep -q 'after the restart' /tmp/restarted.out")

    with subtest("a wait that finishes no scan for 5 minutes fails, after warning of each failed try"):
        waiting("stopped", "15m")
        server.succeed("systemctl stop stalwart")
        stopped = time.time()
        host1.wait_until_succeeds("test -e /tmp/stopped.status", timeout=420)
        took = time.time() - stopped
        err = host1.succeed("cat /tmp/stopped.err")
        print(f"the wait failed {took:.0f} s after Stalwart stopped:\n{err}")
        assert host1.succeed("cat /tmp/stopped.status").strip() != "0", err
        failed = err.index("no scan finished in 5 minutes")
        assert "signing in again in" in err[:failed] and took < 6 * 60, (took, err)
  '';
}
