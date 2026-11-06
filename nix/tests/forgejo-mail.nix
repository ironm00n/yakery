# oracle-a1-flex-1's Forgejo mailing through cx33-1's Stalwart and its program accounts: a mention
# arrives signed as forgejo@, the app password signs in from flex-1's public and overlay addresses,
# and the submission listener refuses mail before sign-in, the app password from another address,
# and a From other than the account's own.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain address;
  signer = pkgs.callPackage ../../crates/rust.nix { crate = "submission-signer"; };
  seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
  sender = address "forgejo";
  passwordFile = "/run/forgejo-smtp-password";
  outsideIp = "192.168.1.250";
  inherit (stalwart.my-lib.netbird.peers) hetzner-cx33-1 oracle-a1-flex-1;
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
        submission.enable = true;
        signSubmissions = true;
      };
      sops.secrets.stalwart-submission_seed.text = seed;
      environment.systemPackages = [ signer ];
    };

  nodes.forgejo =
    { lib, ... }:
    {
      imports = [
        stalwart.client
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
      appPassword = nodes.server.services.stalwart.appPasswords.${sender}.file;
    in
    stalwart.python
    + ''
      import re

      def cli(args):
          return forgejo.succeed(
              "runuser -u forgejo -- ${lib.getExe package} --work-path ${stateDir}"
              f" --config ${customDir}/conf/app.ini {args}"
          )

      def api(token, path, body):
          forgejo.succeed(
              f"curl -sf -H {shlex.quote('Authorization: token ' + token)} -H 'Content-Type: application/json'"
              f" --data {shlex.quote(json.dumps(body))} http://127.0.0.1:3000/api/v1{path}"
          )

      signed_in = "-a PLAIN -au ${sender} -ap \"$(cat ${passwordFile})\""

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

      with subtest("a mention reaches the mentioned user's mailbox, signed as sent by forgejo@"):
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
                      {"ids": ids, "properties": ["from", "blobId"]})["list"]
          assert [[a["email"] for a in m["from"]] for m in mail] == [["${sender}"]], mail
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
          status, out = swaks(f"{signed_in} -f ${sender}")
          print(out)
          assert status == 0, out

      with subtest("the app password signs in over NetBird, from each of flex-1's overlay addresses"):
          server.succeed(
              "ip addr add ${hetzner-cx33-1.v4}/16 dev enp1s0",
              "ip -6 addr add ${hetzner-cx33-1.v6}/64 dev enp1s0 nodad",
          )
          forgejo.succeed(
              "ip addr add ${oracle-a1-flex-1.v4}/16 dev eth1",
              "ip -6 addr add ${oracle-a1-flex-1.v6}/64 dev eth1 nodad",
          )
          for there, here in [("${hetzner-cx33-1.v4}", "${oracle-a1-flex-1.v4}"),
                              ("${hetzner-cx33-1.v6}", "${oracle-a1-flex-1.v6}")]:
              status, out = swaks(f"{signed_in} -li {here} -f ${sender}", server=there)
              print(out)
              assert status == 0, out

      with subtest("the app password is refused from any other address"):
          forgejo.succeed("ip addr add ${outsideIp}/24 dev eth1")
          status, out = swaks(f"{signed_in} -li ${outsideIp} -f ${sender}")
          print(out)
          assert status != 0 and "235 " not in out, out
          server.wait_until_succeeds(
              "journalctl -u stalwart.service -o cat | grep -F security.ip-unauthorized"
              " | grep -qF 'remoteIp = ${outsideIp},'",
              timeout=30,
          )

      with subtest("the account sends only as itself"):
          status, out = swaks(f"{signed_in} -f ${address "owen"}")
          print(out)
          assert status != 0 and "501 5.5.4 You are not allowed to send from this address" in out, out
          status, out = swaks(f"{signed_in} -f ${sender} --h-From ${address "owen"}")
          print(out)
          assert status != 0 and "550 5.7.1 From is not ${sender}, the address this account sends from" in out, out
    '';
}
