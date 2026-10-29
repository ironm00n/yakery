# hetzner-cx33-1's Stalwart in a VM: its stalwart, reverse-proxy and backup bundles, departing from
# the host only in `overrides`. Garage stands in for Hetzner's Object Storage at the bundle's own
# endpoint, behind a switch that answers as nbg1 did on 2026-10-26 while `unavailableFlag` exists.
# A test adds to the node, and no test picks steps out of the bundle's plan.
{ pkgs }:
let
  inherit (pkgs) lib;
  real-lib = import ../../lib { inherit lib; };
  domain = "example.test";
  my-lib = real-lib // {
    ironmoon = lib.mapAttrsRecursive (
      _: v: if lib.isString v then lib.replaceStrings [ real-lib.ironmoon.domain ] [ domain ] v else v
    ) real-lib.ironmoon;
  };
  inherit (my-lib.stalwart) update upsertOn ref;
  inherit (my-lib.ironmoon.mail) loopback;
  jmap = "http://${loopback.address}:${toString loopback.port}";

  hostName = "hetzner-cx33-1";
  recoveryPassword = "recovery";
  unavailableFlag = "/run/s3-unavailable";
  # what nbg1 answered Stalwart with, Code and Message as logged
  unavailable = ''<?xml version="1.0" encoding="UTF-8"?><Error><Code>ServiceUnavailable</Code><Message>Service is unable to handle request.</Message></Error>'';

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

  # every way the node departs from cx33-1
  overrides =
    { tlsHosts, stalwartS3Endpoint }:
    { config, ... }:
    let
      blobStore = s3 config;
      s3Host = lib.removePrefix "https://" blobStore.endpoint;
      vhosts = lib.attrNames config.bundles.reverse-proxy.hosts;
      tls = pkgs.runCommand "stalwart-test-tls" { nativeBuildInputs = [ pkgs.minica ]; } ''
        mkdir $out && cd $out
        ${lib.concatMapStrings (host: "minica --domains ${host}\n") (vhosts ++ tlsHosts ++ [ s3Host ])}
      '';
      certificate = host: {
        sslCertificate = "${tls}/${host}/cert.pem";
        sslCertificateKey = "${tls}/${host}/key.pem";
      };
    in
    {
      # sops-nix stays out of tests; every secret a bundle declares is a file of test data
      imports = [ secretFiles ];
      _module.args = {
        # test data stays off the real domain, and the test network stands in for NetBird at /admin
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
          # the host's addresses are the VM's
          data.ips.${hostName} = {
            ipv4.address = config.networking.primaryIPAddress;
            ipv6 = {
              prefix = "2001:db8:1";
              prefixLength = 64;
            };
          };
        };
      };

      # a VM reaches no ACME server, so nginx serves certificates from a build-time CA
      security.acme.certs = lib.mkForce { };
      security.pki.certificateFiles = [ "${tls}/minica.pem" ];
      services.nginx.virtualHosts =
        lib.genAttrs vhosts (host: certificate host // { enableACME = lib.mkForce false; })
        // lib.genAttrs tlsHosts certificate
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
      # no DNS: every name the node serves, the S3 endpoint's included, is this node
      networking.hosts.${config.networking.primaryIPAddress} = vhosts ++ tlsHosts ++ [ s3Host ];

      # Hetzner's Object Storage is Garage on this node, with the bucket and key the bundle names
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
      services.stalwart.plan.sequence = lib.mkAfter (
        lib.optional (stalwartS3Endpoint != null) (
          update "BlobStore" {
            region = {
              "@type" = "Custom";
              customEndpoint = stalwartS3Endpoint;
              customRegion = blobStore.region;
            };
          }
        )
      );

      # the test script administers Stalwart as the recovery admin, which cx33-1 lets in only to apply a plan
      services.stalwart.recoveryMode = {
        forceEnableUser = true;
        passwordFile = lib.mkForce (pkgs.writeText "recovery-password" recoveryPassword);
      };
    };
in
{
  inherit
    domain
    my-lib
    s3
    unavailableFlag
    ;
  address = user: "${user}@${domain}";

  # `users` are made in the bundle's domain with the role it gives a user; `tlsHosts` are vhosts a
  # test adds outside bundles.reverse-proxy; `stalwartS3Endpoint` puts something between Stalwart
  # and the switch, which stays at `(s3 config).endpoint`
  node =
    {
      users ? [ ],
      tlsHosts ? [ ],
      stalwartS3Endpoint ? null,
    }:
    {
      imports = [
        ../../bundles/stalwart
        ../../bundles/reverse-proxy.nix
        ../../bundles/backup.nix
        (overrides { inherit tlsHosts stalwartS3Endpoint; })
      ];

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
      # a third domain, as cx33-1 has, so the plan is applied with one
      bundles.stalwart.extraDomains.extra = "extra.test";
      services.stalwart.plan.sequence = lib.mkAfter (
        lib.optional (users != [ ]) (
          upsertOn "Account" [ "name" "domainId" ] (
            lib.genAttrs users (name: {
              "@type" = "User";
              inherit name;
              domainId = ref "ironmoon";
              roles."@type" = "User";
              permissions."@type" = "Inherit";
              encryptionAtRest."@type" = "Disabled";
            })
          )
        )
      );
    };

  # another node of the test: it trusts the server's CA and finds the server's vhosts at its address
  client =
    { nodes, ... }:
    {
      security.pki.certificateFiles = nodes.server.security.pki.certificateFiles;
      networking.hosts.${nodes.server.networking.primaryIPAddress} = lib.filter (lib.hasInfix ".") (
        lib.attrNames nodes.server.services.nginx.virtualHosts
      );
    };

  # `server` must be the test's name for a node made by `node` above
  python = ''
    import base64
    import json
    import shlex

    def basic(user, secret):
        return "Basic " + base64.b64encode(f"{user}:{secret}".encode()).decode()

    # the server reaches Stalwart's loopback listener, other nodes go through nginx as on cx33-1
    def origin(node):
        return "${jmap}" if node is server else "https://${my-lib.ironmoon.mail.host}"

    def curl(node, auth, path, body=None):
        data = f"-H 'Content-Type: application/json' --data {shlex.quote(json.dumps(body))}" if body else ""
        out = node.succeed(
            f"curl -s -w '\\n%{{http_code}}' -H {shlex.quote('Authorization: ' + auth)} {data} {origin(node)}{path}"
        )
        payload, status = out.rsplit("\n", 1)
        return int(status), payload

    def session(node, auth):
        status, payload = curl(node, auth, "/jmap/session")
        return status, (json.loads(payload) if status == 200 else None)

    def call(auth, account_id, method, args):
        status, payload = curl(server, auth, "/jmap/", {
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:stalwart:jmap"],
            "methodCalls": [[method, {"accountId": account_id, **args}, "0"]],
        })
        assert status == 200, payload
        name, result, _ = json.loads(payload)["methodResponses"][0]
        assert name == method, result
        return result

    admin = basic("recovery-admin", "${recoveryPassword}")
    admin_id: str
    account_ids: dict[str, str]

    # stalwart-apply-plan links the plan it applied and only then restarts out of recovery mode
    def wait_for_stalwart():
        server.wait_until_succeeds(
            "test -L /var/lib/stalwart/.plan.ndjson"
            " && [ $(systemctl show -P ActiveState stalwart-apply-plan.service) = inactive ]",
            timeout=600,
        )
        assert server.succeed("systemctl show -P Result stalwart-apply-plan.service").strip() == "success"
        server.wait_for_unit("stalwart.service")
        server.wait_until_succeeds("curl -sf ${jmap}/healthz/ready")
        server.wait_until_succeeds(f"curl -sf -H {shlex.quote('Authorization: ' + admin)} ${jmap}/jmap/session")
        global admin_id, account_ids
        admin_id = session(server, admin)[1]["primaryAccounts"]["urn:stalwart:jmap"]
        query = call(admin, admin_id, "x:Account/query", {})
        accounts = call(admin, admin_id, "x:Account/get", {"ids": query["ids"], "properties": ["name"]})
        account_ids = {a["name"]: a["id"] for a in accounts["list"]}

    def app_password(name):
        created = call(admin, admin_id, "x:AppPassword/set", {
            "accountId": account_ids[name],
            "create": {"p": {
                "description": "test",
                "allowedIps": {"127.0.0.1/32": True, "::1/128": True},
                "permissions": {"@type": "Inherit"},
            }},
        })
        assert "p" in created.get("created", {}), created
        return created["created"]["p"]["secret"]
  '';
}
