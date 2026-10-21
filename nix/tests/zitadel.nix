{ pkgs }:
let
  inherit (pkgs) lib;
  my-lib = import ../../lib { inherit lib; };
  host = my-lib.ironmoon.auth.host;
  addr = "192.168.1.1";
  secretsDir = "/etc/zitadel-test";
  scope = "openid profile urn:zitadel:iam:org:project:id:zitadel:aud";
in
pkgs.testers.runNixOSTest {
  name = "zitadel-bundle";

  defaults.virtualisation.graphics = false;

  nodes.machine =
    { ... }:
    {
      imports = [
        ../../modules/zitadel
        ../../bundles/zitadel.nix
        ../../bundles/backup.nix
        ../../bundles/reverse-proxy.nix
        # sops-nix stays out of tests; every secret resolves to a plain file under secretsDir
        {
          options.sops.secrets = lib.mkOption {
            type = lib.types.attrsOf (
              lib.types.submodule (
                { name, ... }:
                {
                  freeformType = lib.types.attrs;
                  options.path = lib.mkOption { default = "${secretsDir}/${name}"; };
                }
              )
            );
          };
        }
      ];
      disabledModules = [ "services/web-apps/zitadel.nix" ];
      _module.args = {
        inherit my-lib;
        inputs.secrets = {
          lib.zitadel = null;
          data.ips.machine = {
            ipv4.address = addr;
            ipv6 = {
              prefix = "2001:db8:1";
              prefixLength = 64;
            };
          };
        };
      };

      virtualisation = {
        memorySize = 2048;
        interfaces.enp1s0 = {
          vlan = 1;
          assignIP = true;
        };
      };

      bundles.zitadel.enable = true;
      bundles.backup.services.zitadel = {
        secretsFile = null;
        passphraseFile = "/dev/null";
        sshKeyFile = "/dev/null";
      };

      environment.etc =
        lib.mapAttrs'
          (
            name: text:
            lib.nameValuePair "zitadel-test/${name}" {
              inherit text;
              mode = "0400";
            }
          )
          {
            zitadel-master_key = "MasterkeyNeedsToHave32Characters";
            # the shape of the real secret: database credentials the generated config has to override
            zitadel-settings = ''
              Database:
                postgres:
                  User:
                    Password: stale
                  Admin:
                    Username: postgres
                    Password: stale
            '';
            zitadel-admin_steps = ''
              FirstInstance:
                PatPath: /tmp/pat
                Org:
                  Machine:
                    Machine:
                      Username: probe
                      Name: probe
                    Pat:
                      ExpirationDate: 2030-01-01T00:00:00Z
            '';
          };
    };

  testScript =
    { nodes, ... }:
    let
      dump = nodes.machine.bundles.backup.services.zitadel.dumpCommand;
    in
    ''
      import base64
      import json

      https = "curl -fsS -k --resolve ${host}:443:${addr} https://${host}"
      ready = "curl -fsS http://127.0.0.1:${toString nodes.machine.services.zitadel.settings.Port}/debug/ready"

      def api(method, path, token, body=None):
          data = f"-d '{json.dumps(body)}' -H 'Content-Type: application/json'" if body is not None else ""
          return json.loads(machine.succeed(f"{https}{path} -X {method} -H 'Authorization: Bearer {token}' {data}"))

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
          pat = machine.succeed("cat /tmp/systemd-private-*-zitadel.service-*/tmp/pat").strip()
          # the first authenticated call answers 503 until the projections behind it catch up
          machine.wait_until_succeeds(f"{https}/auth/v1/users/me -H 'Authorization: Bearer {pat}'", timeout=60)
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
