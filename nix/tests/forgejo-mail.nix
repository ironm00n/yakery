# oracle-a1-flex-1's Forgejo mailing through cx33-1's Stalwart and its program accounts: a mention
# arrives From code@, sent signed in as forgejo@, and a reply to it becomes a comment by the user it
# went to; the app password signs in from flex-1's public and overlay addresses; the submission
# listener refuses mail before sign-in, the app password from another address, and a sender other
# than the account's own addresses; mail to them is refused but at a subaddress of the alias; and an
# address another account already holds makes applying the plan fail.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain address;
  signer = pkgs.callPackage ../../crates/rust.nix { crate = "submission-signer"; };
  seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
  login = address "forgejo";
  sender = address "code";
  passwordFile = "/run/forgejo-smtp-password";
  outsideIp = "192.168.1.250";
  inherit (stalwart.my-lib.netbird.peers) hetzner-cx33-1 oracle-a1-flex-1;
  # both hosts are NetBird peers, on the test network as on nb-netbird, from boot
  overlay = peer: {
    ipv4.addresses = lib.mkAfter [
      {
        address = peer.v4;
        prefixLength = 16;
      }
    ];
    ipv6.addresses = lib.mkAfter [
      {
        address = peer.v6;
        prefixLength = 64;
      }
    ];
  };
in
pkgs.testers.runNixOSTest {
  name = "forgejo-mail";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { nodes, ... }:
    {
      imports = [
        (stalwart.node {
          users = [ "agent" ];
          ips.oracle-a1-flex-1.ipv4.address = nodes.forgejo.networking.primaryIPAddress;
        })
        ../../hosts/hetzner-cx33-1/program-accounts.nix
      ];
      bundles.stalwart = {
        receiving.enable = true;
        submission.enable = true;
        imap.enable = true;
        signSubmissions = true;
      };
      sops.secrets.stalwart-submission_seed.text = seed;
      environment.systemPackages = [
        signer
        pkgs.swaks
      ];
      networking.interfaces.enp1s0 = overlay hetzner-cx33-1;
    };

  nodes.forgejo =
    { lib, nodes, ... }:
    {
      imports = [
        stalwart.secretFiles
        ../../hosts/oracle-a1-flex-1/forgejo.nix
        ../../hosts/oracle-a1-flex-1/postgres.nix
        ../../bundles/reverse-proxy.nix
      ];
      _module.args = {
        inherit (stalwart) my-lib;
        inputs.secrets.lib.forgejo = null;
      };
      virtualisation.memorySize = 2048;
      security.pki.certificateFiles = nodes.server.security.pki.certificateFiles;
      networking.interfaces.eth1 = overlay oracle-a1-flex-1;
      # as NetBird's DNS answers peers
      networking.hosts.${hetzner-cx33-1.v4} = [ stalwart.my-lib.ironmoon.mail.host ];

      # the app password exists only once the server has applied its plan
      sops.secrets.forgejo-smtp_password.path = passwordFile;
      systemd.services.forgejo.wantedBy = lib.mkForce [ ];
      systemd.services.forgejo-zitadel-oauth.enable = false;
      bundles.reverse-proxy.enable = lib.mkForce false;

      environment.systemPackages = [ pkgs.swaks ];
    };

  testScript =
    { nodes, ... }:
    let
      inherit (nodes.forgejo.services.forgejo) package stateDir customDir;
      appPassword = nodes.server.services.stalwart.appPasswords.${login}.file;
    in
    stalwart.python
    + ''
      import re

      def cli(args):
          return forgejo.succeed(
              "runuser -u forgejo -- ${lib.getExe package} --work-path ${stateDir}"
              f" --config ${customDir}/conf/app.ini {args}"
          )

      def api(token, path, body=None):
          data = f" -H 'Content-Type: application/json' --data {shlex.quote(json.dumps(body))}" if body else ""
          return json.loads(forgejo.succeed(
              f"curl -sf -H {shlex.quote('Authorization: token ' + token)}{data} http://127.0.0.1:3000/api/v1{path}"
          ))

      signed_in = "-a PLAIN -au ${login} -ap \"$(cat ${passwordFile})\""
      public, flex1 = "${nodes.server.networking.primaryIPAddress}", "${nodes.forgejo.networking.primaryIPAddress}"

      def swaks(args, tls=True, server="${stalwart.my-lib.ironmoon.mail.host}"):
          starttls = "--tls" if tls else ""
          return forgejo.execute(f"swaks -s {server} -p 587 {starttls} -t ${address "agent"} {args} 2>&1")

      start_all()
      wait_for_stalwart()
      password = server.succeed("cat ${appPassword}").strip()
      forgejo.succeed(f"(umask 077 && echo {shlex.quote(password)} > ${passwordFile})")

      # under TCG initdb can outlast postgresql's start timeout, and its restart finishes the setup
      forgejo.wait_until_succeeds("systemctl is-active postgresql-setup.service", timeout=900)
      forgejo.systemctl("start forgejo.service")
      forgejo.wait_for_unit("forgejo.service")
      forgejo.wait_for_open_port(3000)
      for name in ["owen", "agent"]:
          cli(f"admin user create --username {name} --email {name}@${domain} --random-password --must-change-password=false")
      token = cli("admin user generate-access-token --username owen --scopes all --raw").strip()

      with subtest("a mention reaches the mentioned user's mailbox, From code@ and signed as sent from it"):
          api(token, "/user/repos", {"name": "r"})
          api(token, "/repos/owen/r/issues", {"title": "deploy it", "body": "@agent can you look"})
          agent = sign_in(["agent"])["agent"]
          ids = []
          def arrived(_):
              ids[:] = call(agent, account_ids["agent"], "Email/query",
                            {"filter": {"subject": "deploy it"}})["ids"]
              return bool(ids)
          retry(arrived, timeout_seconds=600)
          mail = call(agent, account_ids["agent"], "Email/get",
                      {"ids": ids, "properties": ["from", "replyTo", "blobId"]})["list"]
          assert [[a["email"] for a in m["from"]] for m in mail] == [["${sender}"]], mail
          [[reply_to]] = [[a["email"] for a in m["replyTo"]] for m in mail]
          assert reply_to.startswith("code+") and reply_to.endswith("@${domain}"), reply_to
          server.succeed(
              f"curl -sf -o /tmp/mention.eml -H {shlex.quote('Authorization: ' + agent)}"
              f" {origin(server)}/jmap/download/{account_ids['agent']}/{mail[0]['blobId']}/m.eml"
          )
          stamp = server.succeed(
              "submission-signer check --public $(submission-signer public --seed ${pkgs.writeText "seed" seed})"
              " < /tmp/mention.eml"
          )
          assert re.fullmatch(r"v=1; account=\S+; sender=${sender}; t=\d+\n", stamp), stamp

      with subtest("sign-in is offered only after STARTTLS"):
          status, out = swaks("--quit-after EHLO", tls=False)
          print(out)
          assert status == 0 and not re.search(r"^<-  250[- ]AUTH", out, re.M), out
          status, out = swaks("--quit-after EHLO")
          print(out)
          assert status == 0 and re.search(r"^<~  250[- ]AUTH .*\bPLAIN\b", out, re.M), out

      with subtest("mail is refused before sign-in, with or without STARTTLS"):
          for tls in [False, True]:
              status, out = swaks("-f ${sender} --quit-after MAIL", tls=tls)
              print(out)
              assert status != 0 and "503 5.5.1 You must authenticate first" in out, out

      with subtest("the app password signs in from flex-1's public address"):
          status, out = swaks(f"{signed_in} -li {flex1} -f ${sender}", server=public)
          print(out)
          assert status == 0, out

      with subtest("the app password signs in over NetBird, from each of flex-1's overlay addresses"):
          for there, here in [("${hetzner-cx33-1.v4}", "${oracle-a1-flex-1.v4}"),
                              ("${hetzner-cx33-1.v6}", "${oracle-a1-flex-1.v6}")]:
              status, out = swaks(f"{signed_in} -li {here} -f ${sender}", server=there)
              print(out)
              assert status == 0, out

      with subtest("the app password is refused from any other address"):
          forgejo.succeed("ip addr add ${outsideIp}/24 dev eth1")
          status, out = swaks(f"{signed_in} -li ${outsideIp} -f ${sender}", server=public)
          print(out)
          assert status != 0 and "235 " not in out, out
          server.wait_until_succeeds(
              "journalctl -u stalwart.service -o cat | grep -F security.ip-unauthorized"
              " | grep -qF 'remoteIp = ${outsideIp},'",
              timeout=30,
          )

      with subtest("the account sends only from its own addresses, not a user's or another account's alias"):
          for other in ["${address "owen"}", "${address "auth"}"]:
              status, out = swaks(f"{signed_in} -f {other}")
              print(out)
              assert status != 0 and "501 5.5.4 You are not allowed to send from this address" in out, out
          status, out = swaks(f"{signed_in} -f ${sender} --h-From ${address "owen"}")
          print(out)
          assert status != 0 and "550 5.7.1 From is not ${sender}, the address this account sends from" in out, out

      with subtest("port 25 refuses mail to the account at its name and its alias, and takes it at a subaddress of the alias"):
          for to, refused in [("${login}", True), ("${sender}", True), ("code+x@${domain}", False)]:
              status, out = server.execute(
                  f"swaks -s {public} -p 25 --ehlo example.org -f someone@example.org"
                  f" -t {to} --quit-after RCPT 2>&1"
              )
              print(out)
              assert (status != 0 and "550 5.1.1 mailbox unavailable" in out) == refused, out

      with subtest("a reply to the mention becomes a comment by the user it went to, and no other mail to code@ does"):
          created = call(admin, admin_id, "x:AppPassword/set", {
              "accountId": account_ids["agent"],
              "create": {"r": {
                  "description": "replies",
                  "allowedIps": {f"{public}/32": True},
                  "permissions": {"@type": "Inherit"},
              }},
          })
          secret = created["created"]["r"]["secret"]

          def reply(to, body):
              return server.execute(
                  f"swaks -s {public} -p 587 --tls"
                  f" -a PLAIN -au ${address "agent"} -ap {shlex.quote(secret)} -f ${address "agent"}"
                  f" -t {to} --h-Subject 'Re: deploy it' --body {shlex.quote(body)} 2>&1"
              )

          for to in ["${login}", "${sender}"]:
              status, out = reply(to, "to a bare address")
              print(out)
              assert status != 0 and "550 5.1.1 mailbox unavailable" in out, out
          status, out = reply("code+garbage@${domain}", "with no token")
          print(out)
          assert status == 0, out
          forgejo.wait_until_succeeds("journalctl -u forgejo.service -o cat | grep -q 'Invalid incoming email token'", timeout=120)
          status, out = reply(reply_to, "on it")
          print(out)
          assert status == 0, out

          def commented(_):
              return [c["body"].strip() for c in api(token, "/repos/owen/r/issues/1/comments") if c["user"]["login"] == "agent"]
          retry(commented, timeout_seconds=120)
          assert commented(None) == ["on it"], commented(None)

      with subtest("an address another account already holds makes applying the plan fail"):
          freed = call(admin, admin_id, "x:Account/set", {"update": {account_ids["forgejo"]: {"aliases": {}}}})
          assert account_ids["forgejo"] in freed.get("updated", {}), freed
          domains = call(admin, admin_id, "x:Domain/get", {
              "ids": call(admin, admin_id, "x:Domain/query", {})["ids"],
              "properties": ["name"],
          })["list"]
          [domain_id] = [d["id"] for d in domains if d["name"] == "${domain}"]
          taken = call(admin, admin_id, "x:Account/set", {"create": {"g": {
              "@type": "Group",
              "name": "code",
              "domainId": domain_id,
              "permissions": {"@type": "Merge", "enabledPermissions": {}},
              "roles": {"@type": "Custom", "roleIds": {}},
          }}})
          assert "g" in taken.get("created", {}), taken
          server.succeed("rm /var/lib/stalwart/.plan.ndjson", "systemctl restart stalwart.service")
          server.wait_until_succeeds(
              "state=$(systemctl show -P ActiveState stalwart-apply-plan.service);"
              " [ $state = failed ] || { [ $state = inactive ] && test -L /var/lib/stalwart/.plan.ndjson; }",
              timeout=600,
          )
          result = server.succeed("systemctl show -P Result stalwart-apply-plan.service").strip()
          applied = server.succeed("journalctl -u stalwart-apply-plan.service -o cat | tail -n 20")
          print(applied)
          assert result == "exit-code" and "primaryKeyViolation" in applied and "Properties: email" in applied, result
    '';
}
