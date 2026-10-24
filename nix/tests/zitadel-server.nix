# A ZITADEL node from bundles/zitadel.nix, with its secrets as plain files under secretsDir, a PAT
# for the first instance's machine user, and Python helpers for its API.
{ pkgs }:
let
  inherit (pkgs) lib;
  my-lib = import ../../lib { inherit lib; };
  secretsDir = "/etc/zitadel-test";
in
{
  host = my-lib.ironmoon.auth.host;
  inherit secretsDir;

  node =
    { config, pkgs, ... }:
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
          data.ips.${config.networking.hostName} = {
            ipv4.address = config.networking.primaryIPAddress;
            ipv6 = {
              prefix = "2001:db8:1";
              prefixLength = 64;
            };
          };
        };
      };

      virtualisation.interfaces.enp1s0 = {
        vlan = 1;
        assignIP = true;
      };

      bundles.zitadel.enable = true;
      # FIXME: packages/protoc-gen-js asks for C++17, which the pinned abseil 20260817 no longer
      # compiles with; drop this once it asks for C++20
      services.zitadel.package = lib.mkForce (
        pkgs.callPackage ../../packages/zitadel {
          callPackage =
            path: args:
            let
              package = pkgs.callPackage path args;
            in
            if path == ../../packages/protoc-gen-js then
              package.overrideAttrs (old: {
                buildPhase = lib.replaceStrings [ "-std=c++17" ] [ "-std=c++20" ] old.buildPhase;
              })
            else
              package;
        }
      );
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

  # `zitadel` must name the node above, and `https` a curl command line ending at its origin
  python = ''
    import json

    def api(method, path, token, body=None):
        data = f"-d '{json.dumps(body)}' -H 'Content-Type: application/json'" if body is not None else ""
        return json.loads(zitadel.succeed(f"{https}{path} -X {method} -H 'Authorization: Bearer {token}' {data}"))

    # the machine user is the instance's IAM owner
    def admin_pat():
        pat = zitadel.succeed("cat /tmp/systemd-private-*-zitadel.service-*/tmp/pat").strip()
        # the first authenticated call answers 503 until the projections behind it catch up
        zitadel.wait_until_succeeds(f"{https}/auth/v1/users/me -H 'Authorization: Bearer {pat}'", timeout=60)
        return pat
  '';
}
