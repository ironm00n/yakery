# Bulwark beside cx33-1's Stalwart from stalwart-server.nix, as the host runs them, with app
# passwords standing in for ZITADEL, and Playwright's specs run in Chromium on the node.
{ pkgs }:
let
  inherit (pkgs) lib;
  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) address unavailableFlag;

  # the accounts the test makes, and root, the bundle's own Admin account
  users = {
    reader = "alice";
    sender = "bob";
  };
  roles = users // {
    admin = "root";
  };
  specs = lib.filter (lib.hasSuffix ".spec.ts") (lib.attrNames (builtins.readDir ./bulwark));
  results = "/tmp/bulwark-e2e";
in
{
  inherit (stalwart) s3 unavailableFlag;

  # `stalwartS3Endpoint` as in stalwart-server.nix's node
  node =
    {
      stalwartS3Endpoint ? null,
    }:
    { config, ... }:
    let
      vhosts = config.bundles.reverse-proxy.hosts;
      webmail = lib.findFirst (host: vhosts.${host}.port == config.services.bulwark.port) null (
        lib.attrNames vhosts
      );
      account = lib.mapAttrs (_: user: {
        address = address user;
        passwordFile = config.services.stalwart.appPasswords.${address user}.file;
      }) roles;
    in
    {
      imports = [
        (stalwart.node {
          users = lib.attrValues users;
          inherit stalwartS3Endpoint;
        })
        ../../bundles/bulwark.nix
      ];

      bundles.bulwark.enable = true;
      services.bulwark.oauth.enabled = lib.mkForce false;
      services.stalwart.appPasswords = lib.genAttrs (map address (lib.attrValues roles)) (_: {
        allowedIps = [ "${config.networking.primaryIPAddress}/32" ];
      });
      systemd.services.bulwark.environment.NODE_EXTRA_CA_CERTS = config.security.pki.caBundle;

      environment.systemPackages = [
        (pkgs.writeShellApplication {
          name = "bulwark-e2e";
          runtimeInputs = [
            pkgs.playwright-test
            pkgs.nssTools
          ];
          runtimeEnv = {
            HOME = "/root";
            NODE_EXTRA_CA_CERTS = config.security.pki.caBundle;
            WEBMAIL_URL = "https://${webmail}";
            JMAP_URL = config.services.bulwark.jmap_server_url;
            ACCOUNTS = pkgs.writeText "accounts.json" (builtins.toJSON account);
            S3_UNAVAILABLE = unavailableFlag;
            inherit (config.systemd.services.bulwark.environment) ADMIN_STATE_DIR;
          };
          # Chromium trusts what its NSS database does, not the system's bundle
          text = ''
            if [ ! -e ~/.pki/nssdb/cert9.db ]; then
              mkdir -p ~/.pki/nssdb
              certutil -d sql:"$HOME/.pki/nssdb" -N --empty-password
              ${lib.concatImapStrings (i: ca: ''
                certutil -d sql:"$HOME/.pki/nssdb" -A -t C,, -n test-ca-${toString i} -i ${ca}
              '') config.security.pki.certificateFiles}
            fi
            OUT=${results}/''${1%.spec.ts} exec playwright test -c ${./bulwark}/playwright.config.ts "$1"
          '';
        })
      ];
    };

  # `server` must be a node made by `node` above
  python =
    server:
    let
      inherit (server.services.bulwark) jmap_server_url port;
    in
    stalwart.python
    + ''
      def wait_for_server():
          wait_for_stalwart()
          server.wait_for_unit("bulwark.service")
          server.wait_until_succeeds(
              "curl -sf ${jmap_server_url}/healthz/ready && curl -sf http://127.0.0.1:${toString port}/api/health"
          )

      # every spec runs, and its results reach the test's output, whichever fail
      def run_specs():
          statuses = {}
          for spec in ${builtins.toJSON specs}:
              statuses[spec], output = server.execute(f"bulwark-e2e {spec} 2>&1", timeout=900)
              print(output)
          server.copy_from_machine("${results}")
          assert not any(statuses.values()), statuses
    '';
}
