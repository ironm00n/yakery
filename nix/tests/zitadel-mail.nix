# hetzner-cx23-2's ZITADEL mailing through cx33-1's Stalwart, signed in as zitadel@ and sending from auth@, over
# IPv6 as a host off NetBird reaches it: zitadel-mail refuses an empty secret and keeps one active SMTP provider however
# often it runs, and the mail ZITADEL sends arrives signed.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  zitadel = import ./zitadel-server.nix {
    inherit pkgs;
    inherit (stalwart) my-lib;
  };
  inherit (stalwart) address;
  signer = pkgs.callPackage ../../crates/rust.nix { crate = "submission-signer"; };
  seed = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
  login = address "zitadel";
  sender = address "auth";
  # each node holds the one IPv6 address its bundle binds, in the /64 both harnesses give their host
  ipv6 = address: {
    networking.interfaces.enp1s0.ipv6.addresses = lib.mkForce [
      {
        inherit address;
        prefixLength = 64;
      }
    ];
  };
  mailAddress = "${stalwart.ipv6Prefix}::2";
  zitadelAddress = "${stalwart.ipv6Prefix}::1:1";
in
pkgs.testers.runNixOSTest {
  name = "zitadel-mail";

  defaults.virtualisation.graphics = false;

  nodes.server =
    { nodes, ... }:
    {
      imports = [
        (stalwart.node {
          users = [ "agent" ];
          ips.hetzner-cx23-2 = {
            ipv4.address = nodes.zitadel.networking.primaryIPAddress;
            ipv6 = {
              prefix = stalwart.ipv6Prefix;
              prefixLength = 64;
            };
          };
        })
        ../../hosts/hetzner-cx33-1/program-accounts.nix
        (ipv6 mailAddress)
      ];
      bundles.stalwart = {
        submission.enable = true;
        signSubmissions = true;
      };
      sops.secrets.stalwart-submission_seed.text = seed;
      environment.systemPackages = [ signer ];
    };

  nodes.zitadel =
    { nodes, ... }:
    {
      imports = [
        zitadel.node
        (ipv6 zitadelAddress)
      ];
      virtualisation.memorySize = 2048;
      bundles.zitadel.mail.enable = true;
      bundles.zitadel.signIn.checkDays = 30;
      # its secrets come from the test: the app password from the server, the token from the first instance
      systemd.services.zitadel-mail.wantedBy = lib.mkForce [ ];
      systemd.services.zitadel-sign-in.wantedBy = lib.mkForce [ ];
      security.pki.certificateFiles = nodes.server.security.pki.certificateFiles;
      # IPv6 only, as cx23-2 reaches mail.ironmoon.dev
      networking.hosts.${mailAddress} = [ stalwart.my-lib.ironmoon.mail.host ];
    };

  testScript =
    { nodes, ... }:
    let
      appPassword = nodes.server.services.stalwart.appPasswords.${login}.file;
      port = toString nodes.zitadel.services.zitadel.settings.Port;
    in
    stalwart.python
    + zitadel.python
    + ''
      import re

      https = "curl -fsS -k --resolve ${zitadel.host}:443:${nodes.zitadel.networking.primaryIPAddress} https://${zitadel.host}"

      start_all()
      wait_for_stalwart()
      zitadel.wait_until_succeeds("curl -fsS http://127.0.0.1:${port}/debug/ready", timeout=600)
      pat = admin_pat()
      secrets = {"api_token": pat, "smtp_password": server.succeed("cat ${appPassword}").strip()}

      def store(name, value):
          zitadel.succeed(f"(umask 077 && printf %s {shlex.quote(value)} > ${zitadel.secretsDir}/zitadel-{name})")

      def ours():
          found = api("POST", "/admin/v1/email/_search", pat, {}).get("result", [])
          return [p for p in found if p.get("description") == "Stalwart, as kept by bundles.zitadel.mail"]

      def email_calls():
          return zitadel.succeed("journalctl -u zitadel.service -o cat | grep -o 'path=/email/[^ ]* status=[0-9]*' || true").splitlines()

      for name, value in secrets.items():
          store(name, value)

      with subtest("zitadel-mail refuses an empty secret before calling ZITADEL, naming its file"):
          for name, value in secrets.items():
              store(name, "")
              before = len(email_calls())
              zitadel.fail("systemctl start zitadel-mail.service")
              # ZITADEL logs this call after any the unit made
              ours()
              retry(lambda _: len(email_calls()) > before, timeout_seconds=60)
              assert email_calls()[before:] == ["path=/email/_search status=200"], email_calls()[before:]
              zitadel.succeed(f"journalctl -u zitadel-mail.service -o cat | grep -Fx '${zitadel.secretsDir}/zitadel-{name} is empty'")
              assert zitadel.succeed("systemctl show -P ActiveState zitadel-mail.service").strip() == "failed"
              store(name, value)

      with subtest("zitadel-mail sets one active provider, and setting it again keeps one"):
          zitadel.succeed("systemctl start zitadel-mail.service")
          zitadel.succeed("systemctl restart zitadel-mail.service")
          [provider] = ours()
          assert provider["state"] == "EMAIL_PROVIDER_ACTIVE", provider
          smtp = provider["smtp"]
          want = {"senderAddress": "${sender}", "user": "${login}", "host": "${stalwart.my-lib.ironmoon.mail.host}:587", "tls": True}
          assert {k: smtp.get(k) for k in want} == want, smtp

      with subtest("zitadel-sign-in refuses an empty token, naming its file"):
          store("api_token", "")
          zitadel.fail("systemctl start zitadel-sign-in.service")
          zitadel.succeed("journalctl -u zitadel-sign-in.service -o cat | grep -Fx '${zitadel.secretsDir}/zitadel-api_token is empty'")
          store("api_token", secrets["api_token"])

      with subtest("zitadel-sign-in sets 30 days on the instance's policy and an org's own, and leaves the rest"):
          lifetimes = ("passwordCheckLifetime", "secondFactorCheckLifetime", "multiFactorCheckLifetime")
          month = ["2592000s"] * 3
          own_org = api("POST", "/management/v1/orgs", pat, {"name": "own policy"})["id"]
          api("POST", "/management/v1/policies/login", pat, {"allowUsernamePassword": True, "disableLoginWithPhone": True}, org=own_org)
          own_before = api("GET", "/management/v1/policies/login", pat, org=own_org)["policy"]
          zitadel.succeed("systemctl start zitadel-sign-in.service")
          instance = api("GET", "/admin/v1/policies/login", pat)["policy"]
          own = api("GET", "/management/v1/policies/login", pat, org=own_org)
          assert [instance[k] for k in lifetimes] == month, instance
          assert not own.get("isDefault") and [own["policy"][k] for k in lifetimes] == month, own
          rest = lambda policy: {k: v for k, v in policy.items() if k not in lifetimes and k != "details"}
          assert rest(own["policy"]) == rest(own_before), (own_before, own["policy"])
          # the token's own org keeps no policy of its own, so it follows the instance's
          followed = api("GET", "/management/v1/policies/login", pat)
          assert followed.get("isDefault") and [followed["policy"][k] for k in lifetimes] == month, followed

      with subtest("zitadel-sign-in started again finds nothing to change and succeeds"):
          zitadel.succeed("systemctl restart zitadel-sign-in.service")
          assert zitadel.succeed("systemctl show -P ActiveState zitadel-sign-in.service").strip() == "active"

      with subtest("the verification code ZITADEL mails a new user arrives over IPv6, From auth@ and signed as sent from it"):
          api("POST", "/v2/users/human", pat, {
              "username": "mail-probe",
              "profile": {"givenName": "Mail", "familyName": "Probe"},
              "email": {"email": "${address "agent"}", "sendCode": {}},
          })
          agent = sign_in(["agent"])["agent"]
          ids = []
          def arrived(_):
              ids[:] = call(agent, account_ids["agent"], "Email/query", {"filter": {"from": "${sender}"}})["ids"]
              return bool(ids)
          retry(arrived, timeout_seconds=300)
          mail = call(agent, account_ids["agent"], "Email/get", {"ids": ids, "properties": ["from", "blobId"]})["list"]
          assert [[a["email"] for a in m["from"]] for m in mail] == [["${sender}"]], mail
          server.succeed(
              f"curl -sf -o /tmp/test.eml -H {shlex.quote('Authorization: ' + agent)}"
              f" {origin(server)}/jmap/download/{account_ids['agent']}/{mail[0]['blobId']}/m.eml"
          )
          stamp = server.succeed(
              "submission-signer check --public $(submission-signer public --seed ${pkgs.writeText "seed" seed})"
              " < /tmp/test.eml"
          )
          assert re.fullmatch(r"v=1; account=\S+; sender=${sender}; t=\d+\n", stamp), stamp
          server.succeed("journalctl -u stalwart.service -o cat | grep -F 'remoteIp = ${stalwart.ipv6Prefix}::'")
    '';
}
