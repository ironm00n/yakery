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
    destroyWhere
    upsertOn
    upsertOn1
    ref
    toSet
    fileSecret
    ;
  cfg = config.bundles.stalwart;
  inherit (my-lib.ironmoon) domain;
  agentDomain = my-lib.ironmoon.mail.agents.domain;
  hostname = my-lib.ironmoon.mail.host;
  inherit (my-lib.ironmoon.mail) loopback;
  httpPort = loopback.port;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  address = "${ips.ipv6.prefix}::2";
  hostnameAddresses = [
    ips.ipv4.address
    address
  ];
  hostAddresses = [
    ips.ipv4.address
    "${ips.ipv6.prefix}::/64"
  ];
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
      (
        [
          "recovery_admin_password"
          "s3_access_key"
          "s3_secret_key"
        ]
        ++ lib.optional cfg.signSubmissions "submission_seed"
      );
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
  }
  // cfg.extraDomains;

  # Stalwart rotates the DKIM keys of each domain in this zone, delegated to Knot on this host
  dkimZone = name: "_domainkey.${name}";
  tsig = {
    keyName = "stalwart";
    algorithm = "hmac-sha512";
    dir = "/var/lib/stalwart-dkim-tsig";
  };

  # the host sessions' account; every account on agentDomain is an agent too
  agentLocalPart = my-lib.ironmoon.mail.agents.sharedLocalPart;

  mailAddress = lib.types.strMatching "[a-z0-9._+-]+@[a-z0-9.-]+";
  accountOf =
    address:
    let
      parts = lib.splitString "@" address;
    in
    {
      local = lib.head (lib.splitString "+" (lib.head parts));
      host = lib.last parts;
    };
  accountAddress =
    address:
    let
      inherit (accountOf address) local host;
    in
    "${local}@${host}";

  # the queue keeps the local part's case
  isSentBy =
    address:
    let
      inherit (accountOf address) local host;
    in
    "sender_domain == '${host}' && (starts_with(to_lowercase(sender), '${local}@') || starts_with(to_lowercase(sender), '${local}+'))";
  isAgentSender = "sender_domain == '${agentDomain}' || (${isSentBy "${agentLocalPart}@${domain}"})";
  isRcpt = rcpt: "to_lowercase(rcpt) == '${rcpt}'";
  agentMayReach = lib.concatStringsSep " || " (
    [
      "rcpt_domain == '${domain}'"
      "ends_with(rcpt_domain, '.${domain}')"
    ]
    ++ map isRcpt cfg.agentCorrespondents
    # the envelope sender is the account's own address, whichever subaddress the From names
    ++ lib.mapAttrsToList (
      agent: correspondents:
      "((${isSentBy agent}) && (${lib.concatMapStringsSep " || " isRcpt correspondents}))"
    ) cfg.agentCorrespondentsFor
  );
  agentRcptForbidden = "(${isAgentSender}) && !(${agentMayReach})";

  # RCPT-stage scripts see only the recipient being added
  unreceivable =
    let
      unreceiving = lib.attrNames (
        lib.filterAttrs (_: account: !account.receivesMail) cfg.programAccounts
      );
    in
    ''
      require ["envelope", "reject"];
      if envelope :all :matches "to" ${sieveList (lib.concatMap serviceAddresses unreceiving)} {
        reject "550 5.1.1 mailbox unavailable";
      }
    '';

  sieveList = strings: "[${lib.concatMapStringsSep ", " builtins.toJSON strings}]";

  # sieve-rs joins the DATA-stage recipients into one string, CRLF between them, and a
  # service mailbox's patterns match its start and end between recipients.
  # A DMARC pass vouches for the From domain; Gmail and the like vouch for its local part.
  inboundGate =
    {
      recipients,
      match ? "matches",
      senders,
      refusal,
    }:
    ''
      if envelope :all :${match} "to" ${sieveList recipients} {
        if not allof (string :is "''${env.dmarc.result}" "pass", address :count "eq" :comparator "i;ascii-numeric" "from" "1", anyof (${lib.concatStringsSep ", " senders})) {
          reject ${builtins.toJSON refusal};
        }
      }
    '';
  anyRecipient = address: [
    address
    "${address}\r\n*"
    "*\r\n${address}"
    "*\r\n${address}\r\n*"
  ];
  serviceAddresses = name: [
    "${name}@${domain}"
    "${name}+*@${domain}"
  ];

  # in the fancy-regex syntax sieve-rs's :regex reads; sieve-rs unescapes \\n, \\r and \\t in a
  # string to control characters, so no backslash precedes those
  agentAddress = "([^@\r\n]*@${lib.escapeRegex agentDomain}|${lib.escapeRegex agentLocalPart}(\\+[^@\r\n]*)?@${lib.escapeRegex domain})";
  alternatives = addresses: lib.concatMapStringsSep "|" lib.escapeRegex addresses;
  # one whole recipient of the CRLF-joined list
  agentRecipient =
    except:
    "(^|\n)${
      lib.optionalString (except != [ ]) "(?!(${alternatives except})(\r|$))"
    }${agentAddress}(\r|$)";
  agentSenders =
    correspondents:
    lib.optional (correspondents != [ ]) ''address :all :is "from" ${sieveList correspondents}''
    ++ [
      ''address :domain :is "from" "${domain}"''
      ''address :domain :matches "from" "*.${domain}"''
    ];
  agentRefusal = "550 5.7.1 agents take mail only from their correspondents";

  grants = lib.concatLists (
    lib.mapAttrsToList (
      agent:
      map (correspondent: {
        inherit agent correspondent;
      })
    ) cfg.agentCorrespondentsFor
  );
  # Stalwart delivers a subaddress to its account, which is all the DATA stage sees of it
  grantsByAccount = lib.mapAttrs (_: map (grant: grant.correspondent)) (
    lib.groupBy (grant: accountAddress grant.agent) grants
  );

  # agent-mail hands a session the mail whose To or Cc names its topic
  grantsByCorrespondent = removeAttrs (lib.mapAttrs (_: map (grant: grant.agent)) (
    lib.groupBy (grant: grant.correspondent) grants
  )) cfg.agentCorrespondents;
  topicGate =
    correspondent: agents:
    let
      anyAgent = agentRecipient [ ];
      otherAgent = "(?i)^(?!(${alternatives agents})$)${agentAddress}$";
    in
    ''
      if allof (envelope :all :regex "to" ${builtins.toJSON anyAgent}, address :all :is "from" ${builtins.toJSON correspondent}, address :all :regex ["to", "cc"] ${builtins.toJSON otherAgent}) {
        reject ${builtins.toJSON agentRefusal};
      }
    '';

  # a message to several agents must pass the gate of each
  inbound = lib.concatStrings (
    [
      ''
        require ["envelope", "reject", "variables", "relational", "comparator-i;ascii-numeric", "regex"];
      ''
      (inboundGate {
        recipients = [ (agentRecipient (lib.attrNames grantsByAccount)) ];
        match = "regex";
        senders = agentSenders cfg.agentCorrespondents;
        refusal = agentRefusal;
      })
    ]
    ++ lib.mapAttrsToList (
      account: correspondents:
      inboundGate {
        recipients = anyRecipient account;
        senders = agentSenders (cfg.agentCorrespondents ++ correspondents);
        refusal = agentRefusal;
      }
    ) grantsByAccount
    ++ lib.mapAttrsToList topicGate grantsByCorrespondent
    ++ lib.mapAttrsToList (
      name: mailbox:
      inboundGate {
        recipients = lib.concatMap anyRecipient (serviceAddresses name);
        senders = [ ''address :domain :is "from" ${builtins.toJSON mailbox.senderDomain}'' ];
        refusal = "550 5.7.1 ${name}@${domain} takes mail only from ${mailbox.senderDomain}";
      }
    ) cfg.serviceMailboxes
  );

  # unless signSubmissions is on, mail an account here submits carries whatever From it writes
  submitted = lib.concatStrings (
    [
      ''
        require ["envelope", "reject", "variables"];
      ''
    ]
    ++ lib.mapAttrsToList (
      name: mailbox:
      let
        address = "${name}@${domain}";
      in
      ''
        if string :is "''${env.authenticated_as}" ${builtins.toJSON address} {
          if not envelope :all :is "to" ${sieveList mailbox.mayMail} {
            reject ${builtins.toJSON "550 5.7.1 ${address} may mail only ${lib.concatStringsSep ", " mailbox.mayMail}"};
          }
        }
        if envelope :all :matches "to" ${sieveList (serviceAddresses name)} {
          reject ${builtins.toJSON "550 5.7.1 ${address} takes mail only from other servers"};
        }
      ''
    ) cfg.serviceMailboxes
  );

  serviceMailboxPermissions = with package.permissions; [
    authenticate
    emailReceive
    emailSend
    jmapMailboxGet
    jmapMailboxQuery
    jmapEmailGet
    jmapEmailQuery
    jmapEmailChanges
    jmapEmailCreate
    jmapEmailUpdate
    jmapEmailDestroy
    jmapIdentityGet
    jmapEmailSubmissionCreate
    jmapEmailSubmissionGet
  ];

  # signed in to by a program, with no role, so it holds only these permissions
  programAccount = name: permissions: {
    "@type" = "User";
    inherit name;
    domainId = ref "ironmoon";
    permissions = {
      "@type" = "Merge";
      enabledPermissions = toSet permissions;
    };
    roles."@type" = "Custom";
    roles.roleIds = [ ];
    encryptionAtRest."@type" = "Disabled";
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
        Its members are the users whose ZITADEL `mail_groups` claim names it.
      '';
    };
    catchAll = lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "[a-z0-9._-]+");
      default = null;
      example = "catchall";
      description = "Shared mailbox that gets mail for addresses at ${domain} no account or alias holds.";
    };
    extraDomains = lib.mkOption {
      type = lib.types.attrsOf (lib.types.strMatching "[a-z0-9.-]+");
      default = { };
      example.dislikes = "dislikes.email";
      description = ''
        Further domains Stalwart takes mail for and signs as, by the id its plan gives each, with
        their own DKIM zone on Knot. Mail reaches only the addresses accounts and aliases hold there.
      '';
    };
    signSubmissions = lib.mkEnableOption ''
      the submission signer: mail an account submits is refused unless its From and Sender are the
      account's own address, and mail with a recipient at ${domain} or ${agentDomain} gets a
      `Submission-Signature` signed with the `submission_seed` secret
    '';
    serviceMailboxes = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            senderDomain = lib.mkOption {
              type = lib.types.strMatching "[a-z0-9.-]+";
              example = "example.edu";
              description = ''
                Mail to the account over port 25 is refused unless it passes DMARC with exactly one
                From address, at this domain and not its subdomains. Accounts here can't send to it.
              '';
            };
            mayMail = lib.mkOption {
              type = lib.types.nonEmptyListOf mailAddress;
              description = "The only addresses the account may send to.";
            };
            allowedIps = lib.mkOption {
              type = lib.types.nonEmptyListOf lib.types.str;
              example = [ "192.0.2.1/32" ];
              description = "Networks its app password may be used from.";
            };
          };
        }
      );
      default = { };
      description = ''
        Accounts at ${domain}, by local part, each for one program: it signs in with the app password
        applying the plan leaves in {option}`services.stalwart.appPasswords.<name>.file`, then reads,
        sends and deletes its own mail over JMAP, and nothing else. Removing one leaves its account
        and app password in Stalwart, without these limits.
      '';
    };
    agentCorrespondents = lib.mkOption {
      type = lib.types.listOf mailAddress;
      default = [ ];
      example = [ "someone@example.com" ];
      description = ''
        Addresses outside ${domain} that ${agentLocalPart}@${domain} and the agents on ${agentDomain} may
        exchange mail with, besides ${domain} and its subdomains. Agents' mail to anyone else is
        refused at RCPT, or bounced when it reaches the queue another way, such as a Sieve redirect.
        Mail to the agents over port 25 is refused unless it passes DMARC as one of these senders.
      '';
    };
    agentCorrespondentsFor = lib.mkOption {
      type = lib.types.attrsOf (lib.types.nonEmptyListOf mailAddress);
      default = { };
      example = {
        "${agentLocalPart}+topic@${domain}" = [ "someone@example.com" ];
      };
      description = ''
        More correspondents, by the agent address that alone may exchange mail with them. Over
        port 25, mail from one reaches only the accounts of the addresses it is listed for, and its
        To and Cc may name no other agent address, since agent-mail hands a session the mail that
        names its topic. Stalwart sees only the account a subaddress belongs to, in either
        direction, so every subaddress of that account may send to them; agent-mail keeps
        ${agentLocalPart}@${domain}'s topics apart.
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
    banExempt = lib.mkOption {
      type = lib.types.attrsOf (lib.types.listOf lib.types.str);
      default = { };
      example = {
        "a campus network" = [ "155.33.0.0/16" ];
      };
      description = ''
        Networks Stalwart never bans, by why: it bans an address after 100 failed sign-ins in a
        day, which locks out every user behind it. This host's own addresses are always exempt.
      '';
    };
    programAccounts = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            permissions = lib.mkOption {
              type = lib.types.uniq (lib.types.listOf (lib.types.enum (lib.attrValues package.permissions)));
              description = "All the account may do: it has no role.";
            };
            allowedIps = lib.mkOption {
              type = lib.types.nonEmptyListOf lib.types.str;
              default = [
                "127.0.0.1/32"
                "::1/128"
              ];
              description = "Networks its app password may be used from.";
            };
            receivesMail = lib.mkEnableOption "mail to the account, which is otherwise refused at RCPT";
            description = lib.mkOption {
              type = lib.types.str;
              default = "program account";
              description = "Its app password's description.";
            };
          };
        }
      );
      default = { };
      description = ''
        Accounts at ${domain}, by local part, each signed in to by one program with the app password
        applying the plan leaves in {option}`services.stalwart.appPasswords.<name>.file`. Removing
        one leaves its account and app password in Stalwart.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.extraDomains ? ironmoon || cfg.extraDomains ? agent);
        message = "bundles.stalwart.extraDomains can't take the ids ironmoon or agent, which the bundle's own domains have";
      }
      {
        assertion = cfg.catchAll == null || lib.elem cfg.catchAll cfg.sharedMailboxes;
        message = "bundles.stalwart.catchAll must be one of bundles.stalwart.sharedMailboxes";
      }
      {
        assertion = lib.all (
          name:
          builtins.match "[a-z0-9._-]+" name != null
          && !lib.elem name (
            [
              "postmaster"
              "root"
              agentLocalPart
            ]
            # the plan's ids for accounts are their local parts, and a domain's id would clash
            ++ lib.attrNames domains
            ++ cfg.sharedMailboxes
          )
        ) (lib.attrNames cfg.programAccounts);
        message = "bundles.stalwart.programAccounts must be local parts no other account at ${domain} has";
      }
      {
        # the addresses agent-mail sends from
        assertion = lib.all (
          agent:
          builtins.match "(${lib.escapeRegex agentLocalPart}\\+[a-z0-9-]+@${lib.escapeRegex domain}|[a-z0-9._-]+(\\+[a-z0-9-]+)?@${lib.escapeRegex agentDomain})" agent
          != null
        ) (lib.attrNames cfg.agentCorrespondentsFor);
        message = "bundles.stalwart.agentCorrespondentsFor must be keyed by topics of ${agentLocalPart}@${domain} or addresses on ${agentDomain}, in lower case";
      }
    ];

    sops.secrets = stalwart-secrets.secrets;

    services.stalwart.submissionSigner = lib.mkIf cfg.signSubmissions {
      enable = true;
      seedFile = get-stalwart-secret "submission_seed";
      domains = lib.attrValues domains;
    };

    bundles.stalwart.programAccounts = {
      backup.permissions = config.services.stalwart.backup.permissions;
    }
    // lib.mapAttrs (_: mailbox: {
      permissions = serviceMailboxPermissions;
      inherit (mailbox) allowedIps;
      receivesMail = true;
      description = "service mailbox";
    }) cfg.serviceMailboxes;

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
        url = "http://${loopback.address}:${toString httpPort}";
      };

      # by default, so a module that makes an account's app password itself, as backup's does, rules
      appPasswords = lib.mapAttrs' (
        name: account:
        lib.nameValuePair "${name}@${domain}" (
          lib.mapAttrs (_: lib.mkDefault) { inherit (account) allowedIps description; }
        )
      ) cfg.programAccounts;

      plan = {
        enableDefaultPlan = true;
        sequence = [
          (reconcile "NetworkListener" [ "name" ] (
            {
              http = {
                name = "http";
                bind = toSet [
                  "${loopback.address}:${toString httpPort}"
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
            }
          ))
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
            issuerUrl = "https://${my-lib.ironmoon.auth.host}";
            requireAudience = webuiClientId;
            claimUsername = "preferred_username";
            claimGroups = "mail_groups";
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
            lib.mapAttrs (_: name: {
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
            }) (lib.genAttrs ([ "postmaster" ] ++ cfg.sharedMailboxes) lib.id)
          ))
          (upsertOn "Account" [ "name" "domainId" ] (
            lib.mapAttrs (name: account: programAccount name account.permissions) cfg.programAccounts
          ))

          (update "MtaOutboundStrategy" {
            route = if cfg.sending.enable then localThenMx else localOnly;
          })
          # unpinned, IPv6 mail leaves from whichever address the kernel prefers, today webmail's
          (reconcile "MtaConnectionStrategy" [ "name" ] {
            default = {
              name = "default";
              sourceIps = map (sourceIp: { inherit sourceIp; }) hostnameAddresses;
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
          # a forged local sender would get the bounce of an undeliverable message, past the inbound script
          (update "MtaStageMail" {
            isSenderAllowed = {
              match = [ ];
              "else" =
                "!is_empty(authenticated_as) || (!key_exists('spam-block', sender_domain) && !is_local_domain(sender_domain))";
            };
          })
          (reconcile "SieveSystemScript" [ "name" ] (
            {
              inbound = {
                name = "inbound";
                description = "Refuses mail to the agents and the service mailboxes from anyone but their senders";
                isActive = true;
                contents = inbound;
              };
              unreceivable = {
                name = "unreceivable";
                description = "Refuses accounts that can't receive, so their bounces can't go to forged senders";
                isActive = true;
                contents = unreceivable;
              };
            }
            // lib.optionalAttrs (cfg.serviceMailboxes != { }) {
              submitted = {
                name = "submitted";
                description = "Keeps each service mailbox to its recipients, and every other account away from it";
                isActive = true;
                contents = submitted;
              };
            }
          ))
          (update "MtaStageRcpt" {
            script = {
              match =
                lib.optional (cfg.serviceMailboxes != { }) {
                  "if" = "!is_empty(authenticated_as)";
                  "then" = "'submitted'";
                }
                ++ [
                  {
                    "if" = "local_port == 25";
                    "then" = "'unreceivable'";
                  }
                ];
              "else" = "false";
            };
          })
          # a user's reject bounces its own text to the sender, past the agent allowlist
          (update "SieveUserInterpreter" {
            disableCapabilities = toSet [
              "reject"
              "ereject"
            ];
          })
          (update "MtaStageData" {
            script = {
              match = [
                {
                  "if" = "local_port == 25";
                  "then" = "'inbound'";
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

          (reconcile "AllowedIp" [ "address" ] (
            lib.listToAttrs (
              lib.imap0
                (i: value: {
                  name = "allowed-${toString i}";
                  inherit value;
                })
                (
                  lib.concatLists (
                    lib.mapAttrsToList (
                      reason:
                      map (address: {
                        inherit address reason;
                      })
                    ) ({ "Bulwark on this host signs in as every user" = hostAddresses; } // cfg.banExempt)
                  )
                )
            )
          ))
        ]
        # bans from before an address was allowed stay listed
        ++ map (ip: destroyWhere "BlockedIp" { address = ip; }) hostnameAddresses;
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
        server.listen = map (ip: "${ip}@53") hostnameAddresses ++ [ "127.0.0.1@53" ];
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
    security.acme.certs.${hostname}.reloadServices =
      lib.optional cfg.receiving.enable "stalwart.service";
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
        listenAddresses = hostnameAddresses;
        listenOnOverlay = true;
        locations = {
          "/admin".netbirdOnly = true;
          "/jmap/eventsource".extraConfig = "proxy_buffering off;";
        };
      };
    };
  };
}
