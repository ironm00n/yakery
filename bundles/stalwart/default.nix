{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  ...
}:
let
  inherit (my-lib.stalwart)
    reconcile
    update
    upsertOn1
    ref
    toSet
    fileSecret
    ;
  cfg = config.bundles.stalwart;
  domain = "ironmoon.dev";
  agentDomain = "agent.${domain}";
  hostname = "mail.${domain}";
  httpPort = 8080;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  address = "${ips.ipv6.prefix}::2";
  webuiClientId = "373369290327392257";

  package = pkgs.callPackage ../../packages/stalwart { webuiOauthClientId = webuiClientId; };

  stalwart-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.stalwart;
        prefix = "stalwart";
        separator = "-";
      }
      [
        "recovery_admin_password"
        "s3_access_key"
        "s3_secret_key"
      ];
  get-stalwart-secret = stalwart-secrets.get-path;
  credential = name: "/run/credentials/stalwart.service/${name}";

  defaults = import ./default-perms.nix {
    inherit lib;
    perms = package.permissions;
  };
  roles = import ./roles.nix {
    perms = package.permissions;
    inherit defaults;
  };

  domainOp =
    id: name:
    upsertOn1 "Domain" id [ "name" ] {
      inherit name;
      dkimManagement."@type" = "Manual";
    };

  localOnly = {
    match = [ ];
    "else" = "'local'";
  };
  localThenMx = {
    match = [
      {
        "if" = "is_local_domain(rcpt_domain)";
        "then" = "'local'";
      }
    ];
    "else" = "'mx'";
  };
in
{
  imports = [ ../../modules/stalwart ];
  disabledModules = [ "services/mail/stalwart.nix" ];

  options.bundles.stalwart = {
    enable = lib.mkEnableOption "Stalwart at ${hostname}, for ${domain} and ${agentDomain}";
    sending.enable = lib.mkEnableOption ''
      delivery to other mail servers. While off, every recipient is routed to local delivery, so
      mail for other domains bounces instead of leaving
    '';
    deliveryWebhooks = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = ''
        URLs Stalwart posts `delivery.completed` events to, with the delivery span's sender and
        recipients, for every message the queue finishes. Webhooks not listed here are removed.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    sops.secrets = stalwart-secrets.secrets;

    services.stalwart = {
      enable = true;
      inherit package;

      recoveryMode = {
        port = 9999;
        passwordFile = get-stalwart-secret "recovery_admin_password";
      };

      credentials = {
        s3_access_key = get-stalwart-secret "s3_access_key";
        s3_secret_key = get-stalwart-secret "s3_secret_key";
      };

      environmentFile = pkgs.writeText "stalwart-env" ''
        STALWART_PUBLIC_URL="https://${hostname}"
      '';
      webui.oauthClientId = webuiClientId;

      plan = {
        enableDefaultPlan = true;
        sequence = [
          (reconcile "NetworkListener" [ "name" ] {
            http = {
              name = "http";
              bind = toSet [
                "127.0.0.1:${toString httpPort}"
                "[::1]:${toString httpPort}"
              ];
              protocol = "http";
              useTls = false;
              tlsImplicit = false;
            };
          })
          (update "Http" {
            useXForwarded = true;
            # Bulwark calls JMAP from its own origin
            usePermissiveCors = true;
          })

          (update "BlobStore" {
            "@type" = "S3";
            bucket = "ironmoon-mail-blobs";
            region = {
              "@type" = "Custom";
              customEndpoint = "https://nbg1.your-objectstorage.com";
              customRegion = "nbg1";
            };
            accessKey = fileSecret (credential "s3_access_key");
            secretKey = fileSecret (credential "s3_secret_key");
            keyPrefix = "trial/";
          })

          (upsertOn1 "Directory" "zitadel-oidc" [ "description" ] {
            "@type" = "Oidc";
            description = "ironmoon Zitadel";
            issuerUrl = "https://auth.${domain}";
            requireAudience = webuiClientId;
            claimUsername = "preferred_username";
            usernameDomain = domain;
            # HACK until https://github.com/zitadel/zitadel/pull/12180 lands
            requireScopes = [ ];
          })
          (reconcile "Role" [ "description" ] (
            lib.mapAttrs (
              _:
              {
                name,
                extends ? [ ],
                enabled ? [ ],
                disabled ? [ ],
              }:
              {
                description = name;
                roleIds = toSet (map ref extends);
                enabledPermissions = toSet enabled;
                disabledPermissions = toSet disabled;
              }
            ) roles
          ))
          (update "Authentication" {
            directoryId = ref "zitadel-oidc";
            defaultUserRoleIds = toSet [ (ref "default-user") ];
            defaultGroupRoleIds = toSet [ (ref "default-group") ];
            defaultAdminRoleIds = toSet [ (ref "super-admin") ];
          })

          (domainOp "ironmoon" domain)
          (domainOp "agent" agentDomain)
          (update "SystemSettings" {
            defaultHostname = hostname;
            defaultDomainId = ref "ironmoon";
          })
          (upsertOn1 "Account" "root" [ "name" "domainId" ] {
            "@type" = "User";
            name = "root";
            domainId = ref "ironmoon";
            permissions."@type" = "Inherit";
            roles."@type" = "Admin";
            encryptionAtRest."@type" = "Disabled";
          })
          (upsertOn1 "Account" "postmaster" [ "name" "domainId" ] {
            "@type" = "Group";
            name = "postmaster";
            domainId = ref "ironmoon";
            permissions."@type" = "Inherit";
            roles."@type" = "Custom";
            roles.roleIds = [ ];
          })

          (update "MtaOutboundStrategy" {
            route = if cfg.sending.enable then localThenMx else localOnly;
          })

          (reconcile "WebHook" [ "url" ] (
            lib.listToAttrs (
              lib.imap0 (i: url: {
                name = "delivery-${toString i}";
                value = {
                  inherit url;
                  enable = true;
                  events = toSet [ "delivery.completed" ];
                  eventsPolicy = "include";
                  lossy = false;
                };
              }) cfg.deliveryWebhooks
            )
          ))
        ];
      };
    };

    networking.interfaces.enp1s0.ipv6.addresses = [
      {
        inherit address;
        prefixLength = ips.ipv6.prefixLength;
      }
    ];

    bundles.reverse-proxy = {
      enable = true;
      openFirewall = true;
      acme-email = "me@${domain}";
      hosts.${hostname} = {
        port = httpPort;
        forwardedFor = "overwrite";
        listenAddresses = [
          ips.ipv4.address
          address
        ];
        listenOnOverlay = true;
        locations = {
          "/admin".netbirdOnly = true;
          "/jmap/eventsource".extraConfig = "proxy_buffering off;";
        };
      };
    };
  };
}
