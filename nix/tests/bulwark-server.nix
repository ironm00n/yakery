# Bulwark and Stalwart from the bundles hetzner-cx33-1 runs, on one node, with app passwords for
# ZITADEL, a build-time CA for ACME, and Garage for Hetzner's Object Storage, behind a switch at the
# blob store's endpoint that answers as nbg1 did on 2026-10-26 while `unavailableFlag` exists.
{ pkgs }:
let
  inherit (pkgs) lib;
  real-lib = import ../../lib { inherit lib; };
  domain = "example.test";
  my-lib = real-lib // {
    ironmoon = lib.mapAttrsRecursive (
      _: lib.replaceStrings [ real-lib.ironmoon.domain ] [ domain ]
    ) real-lib.ironmoon;
  };
  inherit (my-lib.stalwart) update upsertOn ref;

  hostName = "hetzner-cx33-1";
  # the accounts the test makes, and root, the bundle's own Admin account
  users = {
    reader = "alice";
    sender = "bob";
  };
  roles = users // {
    admin = "root";
  };
  address = user: "${user}@${domain}";
  unavailableFlag = "/run/s3-unavailable";
  # what nbg1 answered Stalwart with, Code and Message as logged
  unavailable = ''<?xml version="1.0" encoding="UTF-8"?><Error><Code>ServiceUnavailable</Code><Message>Service is unable to handle request.</Message></Error>'';
  specs = lib.filter (lib.hasSuffix ".spec.ts") (lib.attrNames (builtins.readDir ./bulwark));
  results = "/tmp/bulwark-e2e";

  # The blob store the bundle gives Stalwart, which the switch serves at the same endpoint
  s3 =
    config:
    let
      inherit
        (lib.findFirst (step: step.object == "BlobStore") null config.services.stalwart.plan.sequence)
        value
        ;
    in
    {
      inherit (value) bucket;
      endpoint = value.region.customEndpoint;
      region = value.region.customRegion;
      accessKey = config.sops.secrets.stalwart-s3_access_key.text;
      secretKey = config.sops.secrets.stalwart-s3_secret_key.text;
    };

  # sops-nix stays out of tests; every secret a bundle declares is a file of test data
  secretFiles =
    { config, ... }:
    {
      options.sops.secrets = lib.mkOption {
        type = lib.types.attrsOf (
          lib.types.submodule (
            { name, ... }:
            {
              freeformType = lib.types.attrs;
              options = {
                path = lib.mkOption { default = "/etc/test-secrets/${name}"; };
                text = lib.mkOption { default = builtins.hashString "sha256" name; };
              };
            }
          )
        );
      };
      config.environment.etc = lib.mapAttrs' (
        name: secret:
        lib.nameValuePair "test-secrets/${name}" {
          inherit (secret) text;
          mode = "0400";
          user = lib.defaultTo "root" (secret.owner or null);
          group = lib.defaultTo "root" (secret.group or null);
        }
      ) config.sops.secrets;
    };
in
{
  inherit s3 unavailableFlag;

  # `stalwartS3Endpoint` puts something between Stalwart and the switch, which stays at `(s3 config).endpoint`
  node =
    {
      stalwartS3Endpoint ? null,
    }:
    { config, ... }:
    let
      blobStore = s3 config;
      s3Host = lib.removePrefix "https://" blobStore.endpoint;
      vhosts = config.bundles.reverse-proxy.hosts;
      tls = pkgs.runCommand "bulwark-test-tls" { nativeBuildInputs = [ pkgs.minica ]; } ''
        mkdir $out && cd $out
        ${lib.concatMapStrings (host: "minica --domains ${host}\n") (lib.attrNames vhosts ++ [ s3Host ])}
      '';
      certificate = host: {
        sslCertificate = "${tls}/${host}/cert.pem";
        sslCertificateKey = "${tls}/${host}/key.pem";
      };
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
        ../../bundles/stalwart
        ../../bundles/bulwark.nix
        ../../bundles/reverse-proxy.nix
        ../../bundles/backup.nix
        secretFiles
      ];
      _module.args = {
        # the browser here reaches /admin from the test network, which nginx serves only to NetBird
        my-lib = my-lib // {
          netbird = my-lib.netbird // {
            overlayCidrs = my-lib.netbird.overlayCidrs ++ [ "${config.networking.primaryIPAddress}/32" ];
          };
        };
        inputs.secrets = {
          # sopsFile values, unread with sops-nix out
          lib = {
            stalwart = null;
            bulwark = null;
            borg.stalwart = "/dev/null";
          };
          data.ips.${hostName} = {
            ipv4.address = config.networking.primaryIPAddress;
            ipv6 = {
              prefix = "2001:db8:1";
              prefixLength = 64;
            };
          };
        };
      };

      networking.hostName = hostName;
      virtualisation = {
        memorySize = 4096;
        cores = 4;
        diskSize = 4096;
        interfaces.enp1s0 = {
          vlan = 1;
          assignIP = true;
        };
      };

      bundles.stalwart.enable = true;
      bundles.bulwark.enable = true;

      services.bulwark.oauth.enabled = lib.mkForce false;
      services.stalwart.plan.sequence = lib.mkAfter (
        [
          (upsertOn "Account" [ "name" "domainId" ] (
            lib.genAttrs (lib.attrValues users) (name: {
              "@type" = "User";
              inherit name;
              domainId = ref "ironmoon";
              roles."@type" = "User";
              permissions."@type" = "Inherit";
              encryptionAtRest."@type" = "Disabled";
            })
          ))
        ]
        ++ lib.optional (stalwartS3Endpoint != null) (
          update "BlobStore" {
            region = {
              "@type" = "Custom";
              customEndpoint = stalwartS3Endpoint;
              customRegion = blobStore.region;
            };
          }
        )
      );
      services.stalwart.appPasswords = lib.genAttrs (map address (lib.attrValues roles)) (_: {
        allowedIps = [ "${config.networking.primaryIPAddress}/32" ];
      });

      security.acme.certs = lib.mkForce { };
      security.pki.certificateFiles = [ "${tls}/minica.pem" ];
      networking.hosts.${config.networking.primaryIPAddress} = lib.attrNames vhosts ++ [ s3Host ];
      systemd.services.bulwark.environment.NODE_EXTRA_CA_CERTS = config.security.pki.caBundle;

      services.nginx.virtualHosts =
        lib.mapAttrs (host: _: certificate host // { enableACME = lib.mkForce false; }) vhosts
        // {
          ${s3Host} = certificate s3Host // {
            onlySSL = true;
            # beside the bundles' vhosts, which take this address's connections otherwise
            listenAddresses = [ config.networking.primaryIPAddress ];
            locations."/" = {
              proxyPass = "http://${config.services.garage.settings.s3_api.api_bind_addr}";
              extraConfig = ''
                default_type application/xml;
                if (-e ${unavailableFlag}) {
                  return 503 '${unavailable}';
                }
              '';
            };
          };
        };

      services.garage = {
        enable = true;
        package = pkgs.garage_2;
        settings = {
          replication_factor = 1;
          rpc_bind_addr = "127.0.0.1:3901";
          rpc_public_addr = "127.0.0.1:3901";
          rpc_secret = builtins.hashString "sha256" "garage-rpc";
          s3_api = {
            s3_region = blobStore.region;
            api_bind_addr = "127.0.0.1:3900";
          };
        };
      };
      systemd.services.garage-bucket = {
        description = "Garage's layout, and the bucket and key Stalwart's blob store names";
        requires = [ "garage.service" ];
        after = [ "garage.service" ];
        requiredBy = [ "stalwart.service" ];
        before = [ "stalwart.service" ];
        path = [ config.services.garage.package ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
        };
        script = with blobStore; ''
          until garage status; do sleep 1; done
          garage layout assign -z test -c 1G "$(garage node id -q | cut -d@ -f1)"
          garage layout apply --version 1
          garage key import --yes ${accessKey} ${secretKey}
          garage bucket create ${bucket}
          garage bucket allow --read --write ${bucket} --key ${accessKey}
        '';
      };

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
              certutil -d sql:"$HOME/.pki/nssdb" -A -t C,, -n test-ca -i ${tls}/minica.pem
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
    ''
      def wait_for_server():
          # stalwart-apply-plan links the plan it applied and only then restarts out of recovery mode
          server.wait_until_succeeds(
              "test -L /var/lib/stalwart/.plan.ndjson"
              " && [ $(systemctl show -P ActiveState stalwart-apply-plan.service) = inactive ]",
              timeout=600,
          )
          assert server.succeed("systemctl show -P Result stalwart-apply-plan.service").strip() == "success"
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
