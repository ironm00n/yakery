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
    upsertOn
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
  certDir = config.security.acme.certs.${hostname}.directory;

  defaults = import ./default-perms.nix {
    inherit lib;
    perms = package.permissions;
  };
  roles = import ./roles.nix {
    perms = package.permissions;
    inherit defaults;
  };

  domains = {
    ironmoon = domain;
    agent = agentDomain;
  };

  # Stalwart rotates the DKIM keys of each domain in this zone, delegated to Knot on this host
  dkimZone = name: "_domainkey.${name}";
  tsig = {
    keyName = "stalwart";
    algorithm = "hmac-sha512";
    dir = "/var/lib/stalwart-dkim-tsig";
  };

  # the host sessions' account; every account on agentDomain is an agent too
  agentLocalPart = "agent";

  # the queue keeps the local part's case
  isAgentSender = "sender_domain == '${agentDomain}' || (sender_domain == '${domain}' && (starts_with(to_lowercase(sender), '${agentLocalPart}@') || starts_with(to_lowercase(sender), '${agentLocalPart}+')))";
  agentMayReach = lib.concatStringsSep " || " (
    [
      "rcpt_domain == '${domain}'"
      "ends_with(rcpt_domain, '.${domain}')"
    ]
    ++ map (rcpt: "to_lowercase(rcpt) == '${rcpt}'") cfg.agentCorrespondents
  );
  agentRcptForbidden = "(${isAgentSender}) && !(${agentMayReach})";

  # sieve-rs joins the DATA-stage recipients into one string, hence :matches with * on both sides.
  # A DMARC pass vouches for the From domain; Gmail and the like vouch for its local part.
  agentInbound =
    let
      senders =
        lib.optional (cfg.agentCorrespondents != [ ])
          ''address :all :is "from" [${
            lib.concatMapStringsSep ", " builtins.toJSON cfg.agentCorrespondents
          }]''
        ++ [
          ''address :domain :is "from" "${domain}"''
          ''address :domain :matches "from" "*.${domain}"''
        ];
    in
    ''
      require ["envelope", "reject", "variables", "relational", "comparator-i;ascii-numeric"];
      if envelope :all :matches "to" ["*@${agentDomain}*", "*${agentLocalPart}@${domain}*", "*${agentLocalPart}+*@${domain}*"] {
        if not allof (string :is "''${env.dmarc.result}" "pass", address :count "eq" :comparator "i;ascii-numeric" "from" "1", anyof (${lib.concatStringsSep ", " senders})) {
          reject "550 5.7.1 agents take mail only from their correspondents";
        }
      }
    '';

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
      # Sieve redirects and vacation replies reach the queue without the RCPT check
      {
        "if" = agentRcptForbidden;
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
    sharedMailboxes = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching "[a-z0-9._-]+");
      default = [ ];
      example = [ "catchall" ];
      description = ''
        Groups at ${domain}, each a shared mailbox, besides postmaster.
      '';
    };
    catchAll = lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "[a-z0-9._-]+");
      default = null;
      example = "catchall";
      description = "Shared mailbox that gets mail for addresses at ${domain} no account or alias holds.";
    };
    agentCorrespondents = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching "[a-z0-9._+-]+@[a-z0-9.-]+");
      default = [ ];
      example = [ "someone@example.com" ];
      description = ''
        Addresses outside ${domain} that agent@${domain} and the agents on ${agentDomain} may
        exchange mail with, besides ${domain} and its subdomains. Agents' mail to anyone else is
        refused at RCPT, or bounced when it reaches the queue another way, such as a Sieve redirect.
        Mail to the agents over port 25 is refused unless it passes DMARC as one of these senders.
      '';
    };
    receiving.enable = lib.mkEnableOption ''
      SMTP on port 25 of the public addresses, with STARTTLS on the ACME certificate nginx keeps for
      ${hostname}, so mail from other servers reaches the local domains
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
    assertions = [
      {
        assertion = cfg.catchAll == null || lib.elem cfg.catchAll cfg.sharedMailboxes;
        message = "bundles.stalwart.catchAll must be one of bundles.stalwart.sharedMailboxes";
      }
    ];

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
        dkim_tsig = "${tsig.dir}/secret";
      }
      // lib.optionalAttrs cfg.receiving.enable {
        tls_certificate = "${certDir}/fullchain.pem";
        tls_key = "${certDir}/key.pem";
      };

      environmentFile = pkgs.writeText "stalwart-env" ''
        STALWART_PUBLIC_URL="https://${hostname}"
      '';
      webui.oauthClientId = webuiClientId;

      backup = {
        enable = true;
        package = pkgs.callPackage ../../packages/vandelay { };
        account = "backup@${domain}";
        url = "http://127.0.0.1:${toString httpPort}";
      };

      plan = {
        enableDefaultPlan = true;
        sequence = [
          (reconcile "NetworkListener" [ "name" ] ({
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
          }
          // lib.optionalAttrs cfg.receiving.enable {
            smtp = {
              name = "smtp";
              bind = toSet [
                "${ips.ipv4.address}:25"
                "[${address}]:25"
              ];
              protocol = "smtp";
              useTls = true;
              tlsImplicit = false;
            };
          }))
          (reconcile "Certificate" [ "certificate" ] (
            lib.optionalAttrs cfg.receiving.enable {
              mail = {
                certificate = fileSecret (credential "tls_certificate");
                privateKey = fileSecret (credential "tls_key");
              };
            }
          ))
          (update "Http" {
            useXForwarded = true;
            # Bulwark calls JMAP from its own origin
            usePermissiveCors = true;
          })

          # Claude sessions all sign in as agent@, and each one waiting for mail holds a request open
          (update "Jmap" {
            maxConcurrentRequests = 32;
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
            keyPrefix = null;
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

          (reconcile "DnsServer" [ "description" ] {
            knot = {
              "@type" = "Tsig";
              description = "Knot on this host, for the delegated _domainkey zones";
              host = "127.0.0.1";
              inherit (tsig) keyName;
              key = fileSecret (credential "dkim_tsig");
              tsigAlgorithm = tsig.algorithm;
            };
          })
          (upsertOn "Domain" [ "name" ] (
            lib.recursiveUpdate (lib.mapAttrs (_: name: {
              inherit name;
              dkimManagement = {
                "@type" = "Automatic";
                algorithms = toSet [
                  "Dkim1RsaSha256"
                  "Dkim1Ed25519Sha256"
                ];
              };
              dnsManagement = {
                "@type" = "Automatic";
                dnsServerId = ref "knot";
                origin = dkimZone name;
                publishRecords = toSet [ "dkim" ];
              };
            }) domains) { ironmoon.catchAllAddress = lib.mapNullable (g: "${g}@${domain}") cfg.catchAll; }
          ))
          (update "SystemSettings" {
            defaultHostname = hostname;
            defaultDomainId = ref "ironmoon";
            # SMTP clients often send no SNI
            defaultCertificateId = if cfg.receiving.enable then ref "mail" else null;
          })
          (upsertOn1 "Account" "root" [ "name" "domainId" ] {
            "@type" = "User";
            name = "root";
            domainId = ref "ironmoon";
            permissions."@type" = "Inherit";
            roles."@type" = "Admin";
            encryptionAtRest."@type" = "Disabled";
          })
          (upsertOn "Account" [ "name" "domainId" ] (
            lib.mapAttrs
              (_: name: {
                "@type" = "Group";
                inherit name;
                domainId = ref "ironmoon";
                # with no role, delivery to the group fails and bounces
                permissions = {
                  "@type" = "Merge";
                  enabledPermissions = toSet [ "emailReceive" ];
                };
                roles."@type" = "Custom";
                roles.roleIds = [ ];
              })
              (lib.genAttrs ([ "postmaster" ] ++ cfg.sharedMailboxes) lib.id)
          ))
          (upsertOn1 "Account" "backup" [ "name" "domainId" ] {
            "@type" = "User";
            name = "backup";
            domainId = ref "ironmoon";
            permissions = {
              "@type" = "Merge";
              enabledPermissions = toSet config.services.stalwart.backup.permissions;
            };
            roles."@type" = "Custom";
            roles.roleIds = [ ];
            encryptionAtRest."@type" = "Disabled";
          })

          (update "MtaOutboundStrategy" {
            route = if cfg.sending.enable then localThenMx else localOnly;
          })
          # unpinned, IPv6 mail leaves from whichever address the kernel prefers, today webmail's
          (reconcile "MtaConnectionStrategy" [ "name" ] {
            default = {
              name = "default";
              sourceIps = map (sourceIp: { inherit sourceIp; }) [
                ips.ipv4.address
                address
              ];
            };
          })
          (update "MtaStageRcpt" {
            allowRelaying = {
              match = [
                {
                  "if" = "is_empty(authenticated_as)";
                  "then" = "false";
                }
                {
                  "if" = agentRcptForbidden;
                  "then" = "false";
                }
              ];
              "else" = "true";
            };
          })
          # a forged local sender would get the bounce of an undeliverable message, past agent-inbound
          (update "MtaStageMail" {
            isSenderAllowed = {
              match = [ ];
              "else" =
                "!is_empty(authenticated_as) || (!key_exists('spam-block', sender_domain) && !is_local_domain(sender_domain))";
            };
          })
          (reconcile "SieveSystemScript" [ "name" ] {
            agent-inbound = {
              name = "agent-inbound";
              description = "Refuses mail to ${agentDomain} from anyone but its correspondents";
              isActive = true;
              contents = agentInbound;
            };
          })
          (update "MtaStageData" {
            script = {
              match = [
                {
                  "if" = "local_port == 25";
                  "then" = "'agent-inbound'";
                }
              ];
              "else" = "false";
            };
          })

          # relaxed, Stalwart's default on port 25, only records a failed p=reject
          (update "SenderAuth" {
            dmarcVerify = {
              match = [
                {
                  "if" = "local_port == 25";
                  "then" = "strict";
                }
              ];
              "else" = "disable";
            };
          })

          # Stalwart re-creates both default throttles whenever none exist,
          # so the one turned off stays as a disabled object
          (reconcile "MtaInboundThrottle" [ "description" ] {
            remote-ip = {
              description = "Sender IP throttle";
              enable = true;
              key = toSet [ "remoteIp" ];
              rate = {
                count = 5;
                period = 1000;
              };
            };
            sender-domain-to-rcpt = {
              description = "Sender address to recipient throttle";
              enable = false;
              key = toSet [
                "senderDomain"
                "rcpt"
              ];
              rate = {
                count = 25;
                period = 60 * 60 * 1000;
              };
            };
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

    bundles.backup.services.stalwart = {
      inherit (config.services.stalwart.backup) dumpCommand;
      readWritePaths = [ config.services.stalwart.backup.stateDir ];
    };

    networking.interfaces.enp1s0.ipv6.addresses = [
      {
        inherit address;
        prefixLength = ips.ipv6.prefixLength;
      }
    ];

    services.knot = {
      enable = true;
      keyFiles = [ "/run/credentials/knot.service/tsig.conf" ];
      settings = {
        server.listen = [
          "${ips.ipv4.address}@53"
          "${address}@53"
          "127.0.0.1@53"
        ];
        acl = [
          {
            id = "stalwart";
            address = [ "127.0.0.1" ];
            key = tsig.keyName;
            action = "update";
          }
        ];
        mod-rrl = [ { id = "default"; } ];
        template = [
          {
            id = "default";
            global-module = "mod-rrl/default";
            # the store's zone files hold SOA and NS; Stalwart's TXT records live only in the journal
            zonefile-sync = -1;
            zonefile-load = "difference-no-serial";
            journal-content = "all";
            zonefile-skip = [ "TXT" ];
          }
        ];
        zone = lib.mapAttrsToList (_: name: {
          domain = dkimZone name;
          file = pkgs.writeText "${dkimZone name}.zone" ''
            @ 3600 SOA ${hostname}. hostmaster.${domain}. 1 3600 900 1209600 300
            @ 3600 NS ${hostname}.
          '';
          acl = "stalwart";
        }) domains;
      };
    };
    systemd.services.knot.serviceConfig.LoadCredential = "tsig.conf:${tsig.dir}/knot.conf";
    systemd.services.stalwart-dkim-tsig = {
      description = "TSIG key Stalwart signs its DKIM updates to Knot with";
      before = [
        "knot.service"
        "stalwart.service"
      ];
      requiredBy = [
        "knot.service"
        "stalwart.service"
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        StateDirectory = baseNameOf tsig.dir;
        UMask = "0077";
      };
      script = ''
        cd ${tsig.dir}
        if [ ! -s secret ]; then
          head -c 64 /dev/urandom | base64 -w0 > secret.new
          mv secret.new secret
        fi
        printf 'key:\n  - id: %s\n    algorithm: %s\n    secret: %s\n' \
          ${tsig.keyName} ${tsig.algorithm} "$(cat secret)" > knot.conf.new
        mv knot.conf.new knot.conf
      '';
    };

    networking.firewall.allowedUDPPorts = [ 53 ];
    networking.firewall.allowedTCPPorts = [ 53 ] ++ lib.optional cfg.receiving.enable 25;
    # Stalwart reads the certificate files only at start
    security.acme.certs.${hostname}.reloadServices = lib.optional cfg.receiving.enable "stalwart.service";
    systemd.services.stalwart = lib.mkIf cfg.receiving.enable {
      wants = [ "acme-${hostname}.service" ];
      after = [ "acme-${hostname}.service" ];
    };

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
