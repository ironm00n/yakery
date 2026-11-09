# hetzner-cx23-2's ZITADEL mailing through cx33-1's Stalwart, signed in as zitadel@ and sending from auth@, over
# IPv6 as a host off NetBird reaches it: zitadel-mail keeps one active SMTP provider however often it runs, and the
# mail ZITADEL sends arrives signed.
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
      # its secrets come from the test: the app password from the server, the token from the first instance
      systemd.services.zitadel-mail.wantedBy = lib.mkForce [ ];
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
      password = server.succeed("cat ${appPassword}").strip()
      zitadel.succeed(
          f"(umask 077 && echo {shlex.quote(password)} > ${zitadel.secretsDir}/zitadel-smtp_password"
          f" && echo {shlex.quote(pat)} > ${zitadel.secretsDir}/zitadel-api_token)"
      )

      def ours():
          found = api("POST", "/admin/v1/email/_search", pat, {}).get("result", [])
          return [p for p in found if p.get("description") == "Stalwart, as kept by bundles.zitadel.mail"]

      with subtest("zitadel-mail sets one active provider, and setting it again keeps one"):
          zitadel.succeed("systemctl start zitadel-mail.service")
          zitadel.succeed("systemctl restart zitadel-mail.service")
          [provider] = ours()
          assert provider["state"] == "EMAIL_PROVIDER_ACTIVE", provider
          smtp = provider["smtp"]
          want = {"senderAddress": "${sender}", "user": "${login}", "host": "${stalwart.my-lib.ironmoon.mail.host}:587", "tls": True}
          assert {k: smtp.get(k) for k in want} == want, smtp

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
