# Local ACME CA (step-ca) issuing trusted certs.
#
# The root CA is created once, out of band. Each machine mints its own
# intermediate from it on first boot.
{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkOption
    mkIf
    mkForce
    mkDefault
    types
    mapAttrs'
    nameValuePair
    ;
  cfg = config.bundles.local-tls;

  # Public root cert (safe for Nix store).
  caCertFile = pkgs.writeText "local-tls-root-ca.crt" cfg.ca.certPem;

  stateDir = "/var/lib/step-ca";
  intermediateCrt = "${stateDir}/intermediate_ca.crt";
  intermediateKey = "${stateDir}/intermediate_ca.key";

  acmeDirectory = "https://${cfg.caDomain}:${toString cfg.stepCaPort}/acme/acme/directory";

  caSecrets = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.root-ca;
    prefix = "local-tls";
    usergroup = "step-ca";
  } [ "ca_private_key" ];
  rootKeyPath = caSecrets.get-path "ca_private_key";
in
{
  options.bundles.local-tls = {
    enable = mkEnableOption "a local ACME CA (step-ca) issuing trusted certs.";

    caName = mkOption {
      type = types.str;
      default = "ironmoon Local CA";
      description = "Human-readable name used for the minted intermediate CA.";
    };

    caDomain = mkOption {
      type = types.str;
      default = "ca.test";
      description = "Hostname the step-ca ACME server is reached at (resolved to loopback).";
    };

    stepCaPort = mkOption {
      type = types.port;
      default = 8443;
      description = "Port step-ca listens on (loopback only).";
    };

    domains = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [
        "app.test"
        "api.test"
      ];
      description = ''
        Domains resolved to loopback and expected to obtain a certificate from
        the local CA.
      '';
    };

    ca.certPem = mkOption {
      type = types.str;
      default = inputs.secrets.lib.root-ca-pub;
      defaultText = lib.literalExpression "inputs.secrets.lib.root-ca-pub";
      description = "PEM-encoded root CA certificate (public; safe for the Nix store).";
    };

    trust = {
      system = mkOption {
        type = types.bool;
        default = true;
        description = "Add the root CA to the system trust store.";
      };
      firefox = mkOption {
        type = types.bool;
        default = true;
        description = "Trust the root CA in Firefox (via the enterprise-roots policy).";
      };
    };

    caCert = mkOption {
      type = types.str;
      readOnly = true;
      description = ''
        Resolved path of the root CA certificate, for things like
        NODE_EXTRA_CA_CERTS. Also available at /etc/local-tls/ca.crt.
      '';
    };
  };

  config = mkIf cfg.enable {
    bundles.local-tls.caCert = "${caCertFile}";

    networking.hosts = {
      "127.0.0.1" = [ cfg.caDomain ] ++ cfg.domains;
      "::1" = [ cfg.caDomain ] ++ cfg.domains;
    };

    # Static user (not DynamicUser) so bootstrap and daemon share /var/lib/step-ca.
    users.users.step-ca = {
      isSystemUser = true;
      group = "step-ca";
      home = stateDir;
    };
    users.groups.step-ca = { };

    sops.secrets = caSecrets.secrets;

    # The local ACME CA.
    services.step-ca = {
      enable = true;
      address = "127.0.0.1";
      port = cfg.stepCaPort;
      settings = {
        root = "${caCertFile}";
        crt = intermediateCrt;
        key = intermediateKey;
        dnsNames = [
          cfg.caDomain
          "localhost"
          "127.0.0.1"
        ];
        logger.format = "text";
        db = {
          type = "badgerv2";
          dataSource = "${stateDir}/db";
        };
        authority.provisioners = [
          {
            type = "ACME";
            name = "acme";
            # ~90 days, mirroring Let's Encrypt so renewal behaves like prod.
            claims = {
              defaultTLSCertDuration = "2160h";
              maxTLSCertDuration = "2160h";
            };
          }
        ];
      };
    };

    systemd.services = {
      step-ca.serviceConfig = {
        DynamicUser = mkForce false;
        User = "step-ca";
        Group = "step-ca";
      };

      # Mint the intermediate from the sops root key, once per machine. Its key
      # is written unencrypted (0600 via UMask, in root-only /var/lib/step-ca).
      local-tls-ca-setup = {
        description = "Bootstrap the local-tls intermediate CA";
        before = [ "step-ca.service" ];
        requiredBy = [ "step-ca.service" ];
        path = [ pkgs.step-cli ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          User = "step-ca";
          Group = "step-ca";
          StateDirectory = "step-ca";
          UMask = "0077";
        };
        script = ''
          set -euo pipefail
          if [ ! -s ${intermediateCrt} ] || [ ! -s ${intermediateKey} ]; then
            echo "Minting local-tls intermediate CA..."
            step certificate create "${cfg.caName} Intermediate" \
              ${intermediateCrt} ${intermediateKey} \
              --profile intermediate-ca \
              --ca ${caCertFile} --ca-key ${rootKeyPath} \
              --no-password --insecure \
              --not-after 43800h
          fi
        '';
      };
    }
    // mapAttrs' (
      name: _:
      nameValuePair "acme-${name}" {
        after = [ "step-ca.service" ];
        wants = [ "step-ca.service" ];
      }
    ) config.security.acme.certs;

    security.acme = {
      acceptTerms = mkDefault true;
      defaults = {
        email = mkDefault "local-tls@${cfg.caDomain}";
        server = acmeDirectory;
      };
    };

    security.pki.certificateFiles = mkIf cfg.trust.system [ caCertFile ];

    programs.firefox.policies = mkIf cfg.trust.firefox {
      Certificates = {
        ImportEnterpriseRoots = true;
        Install = [ "${caCertFile}" ];
      };
    };

    environment.etc."local-tls/ca.crt".source = caCertFile;
  };
}
