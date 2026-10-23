{ pkgs }:
let
  zitadel = import ./zitadel-server.nix { inherit pkgs; };
  inherit (zitadel) host secretsDir;
  addr = "192.168.1.1";
  scope = "openid profile urn:zitadel:iam:org:project:id:zitadel:aud";
in
pkgs.testers.runNixOSTest {
  name = "zitadel-bundle";

  defaults.virtualisation.graphics = false;

  nodes.machine = {
    imports = [ zitadel.node ];
    virtualisation.memorySize = 2048;
  };

  testScript =
    { nodes, ... }:
    let
      dump = nodes.machine.bundles.backup.services.zitadel.dumpCommand;
    in
    zitadel.python
    + ''
      import base64

      zitadel = machine
      https = "curl -fsS -k --resolve ${host}:443:${addr} https://${host}"
      ready = "curl -fsS http://127.0.0.1:${toString nodes.machine.services.zitadel.settings.Port}/debug/ready"

      machine.wait_for_unit("zitadel.service")
      machine.wait_until_succeeds(ready, timeout=300)
      machine.wait_for_unit("nginx.service")

      with subtest("the sandboxed unit serves through the proxy"):
          discovery = json.loads(machine.succeed(f"{https}/.well-known/openid-configuration"))
          assert discovery["issuer"] == "https://${host}", discovery["issuer"]
          keys = json.loads(machine.succeed(f"{https}/oauth/v2/keys"))["keys"]
          assert len(keys) > 0, keys
          machine.succeed(f"{https}/ui/console/ | grep -q '<cnsl-root'")
          machine.succeed(f"{https}/ui/login/resources/themes/zitadel/css/zitadel.css -o /dev/null")

      with subtest("zitadel reaches postgres only as its own role, despite the secret's stale credentials"):
          roles = machine.succeed(
              "runuser -u postgres -- psql -tAc \"select distinct usename from pg_stat_activity where datname = 'zitadel'\""
          ).split()
          assert roles == ["zitadel"], roles

      with subtest("a client-credentials JWT carries the requested scopes"):
          pat = admin_pat()
          uid = api("GET", "/auth/v1/users/me", pat)["user"]["id"]
          api("PUT", f"/management/v1/users/{uid}/machine", pat, {"name": "probe", "accessTokenType": "ACCESS_TOKEN_TYPE_JWT"})
          secret = api("PUT", f"/management/v1/users/{uid}/secret", pat, {})
          token = json.loads(machine.succeed(
              f"{https}/oauth/v2/token -u '{secret['clientId']}:{secret['clientSecret']}'"
              " -d grant_type=client_credentials --data-urlencode 'scope=${scope}'"
          ))["access_token"]
          payload = token.split(".")[1]
          claims = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
          assert claims.get("scope") == "${scope}", claims

      with subtest("the backup dump connects over the socket and restores"):
          machine.succeed(
              "${dump} > /tmp/zitadel.dump",
              "runuser -u postgres -- createdb restored",
              "runuser -u postgres -- pg_restore --no-owner --dbname=restored /tmp/zitadel.dump",
          )
          events = int(machine.succeed("runuser -u postgres -- psql restored -tAc 'select count(*) from eventstore.events2'"))
          assert events > 0, events

      with subtest("a missing secret fails the unit instead of starting on defaults"):
          machine.succeed("systemctl stop zitadel", "mv ${secretsDir}/zitadel-settings /root/settings")
          machine.fail("systemctl start zitadel")
          machine.succeed("journalctl -u zitadel -b | grep -i credential")
          machine.succeed("mv /root/settings ${secretsDir}/zitadel-settings", "systemctl start zitadel")
          machine.wait_until_succeeds(ready, timeout=120)

      with subtest("exposure"):
          print(machine.succeed("systemd-analyze security --no-pager zitadel.service | tail -1"))
    '';
}
