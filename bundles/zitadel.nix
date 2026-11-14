{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  ...
}:
let
  cfg = config.bundles.zitadel;
  hostname = my-lib.ironmoon.auth.host;
  port = 39995;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  zitadelAddr = "${ips.ipv6.prefix}::1:1";
  # the units below call the admin API with an IAM owner's token
  adminApi = cfg.mail.enable || cfg.signIn.checkDays != null;
  zitadel-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.zitadel;
        prefix = "zitadel";
        separator = "-";
      }
      (
        [
          "master_key"
          "admin_steps"
          "settings"
        ]
        # the app password of Stalwart's zitadel@ account
        ++ lib.optional cfg.mail.enable "smtp_password"
        ++ lib.optional adminApi "api_token"
      );
  get-zitadel-secret = zitadel-secrets.get-path;

  # the provider this host keeps, found again by its description
  mailProvider = {
    senderAddress = "${my-lib.ironmoon.mail.senders.zitadel}@${my-lib.ironmoon.domain}";
    senderName = "ZITADEL";
    user = "zitadel@${my-lib.ironmoon.domain}";
    # ZITADEL tries implicit TLS and falls back to STARTTLS; false would sign in in the clear
    tls = true;
    host = "${my-lib.ironmoon.mail.host}:587";
    description = "Stalwart, as kept by bundles.zitadel.mail";
  };
  # EX_CONFIG, which no retry fixes: the deploy that fills the secret starts the unit again
  emptySecret = 78;
  # a oneshot calling ZITADEL's APIs as `api PATH CURL-ARGS...` with the token among its credentials, again at each
  # start; the admin API answers once ZITADEL's projections have caught up after a start
  adminUnit =
    {
      name,
      description,
      credentials,
      text,
    }:
    {
      inherit description;
      requires = [ "zitadel.service" ];
      after = [ "zitadel.service" ];
      wantedBy = [ "multi-user.target" ];
      startLimitIntervalSec = 0;
      serviceConfig = my-lib.systemd.hardening.network // {
        Type = "oneshot";
        RemainAfterExit = true;
        DynamicUser = true;
        Restart = "on-failure";
        RestartSec = "10s";
        RestartPreventExitStatus = emptySecret;
        LoadCredential = lib.mapAttrsToList (name: path: "${name}:${path}") credentials;
        ExecStart = lib.getExe (
          pkgs.writeShellApplication {
            inherit name;
            runtimeInputs = [
              pkgs.curl
              pkgs.jq
            ];
            text = ''
              ${lib.concatMapAttrsStringSep "" (name: path: ''
                [ -n "$(<"$CREDENTIALS_DIRECTORY/${name}")" ] || { echo ${lib.escapeShellArg "${path} is empty"} >&2; exit ${toString emptySecret}; }
              '') credentials}
              # the credentials reach neither argv nor the environment
              api() {
                curl --silent --show-error --fail-with-body --max-time 30 \
                  --config <(printf 'header = "Authorization: Bearer %s"\n' "$(<"$CREDENTIALS_DIRECTORY/token")") \
                  --header 'Host: ${hostname}' --header 'Content-Type: application/json' \
                  "http://127.0.0.1:${toString port}$1" "''${@:2}"
              }
              ${text}
            '';
          }
        );
      };
    };
  token = get-zitadel-secret "api_token";

  setMailProvider = ''
    password() {
      jq --null-input --rawfile p "$CREDENTIALS_DIRECTORY/password" '{password: ($p | rtrimstr("\n"))}'
    }
    want=${lib.escapeShellArg (builtins.toJSON mailProvider)}

    have=$(api /admin/v1/email/_search --data '{}' | jq --compact-output --argjson want "$want" \
      '[.result[]? | select(.description == $want.description)] | first // empty')
    if [ -z "$have" ]; then
      id=$(password | jq --argjson want "$want" '$want + .' | api /admin/v1/email/smtp --data @- | jq --raw-output .id)
    else
      id=$(jq --raw-output .id <<<"$have")
      if [ "$(jq --argjson want "$want" '.smtp | {senderAddress, senderName, user, tls, host} == ($want | del(.description))' <<<"$have")" != true ]; then
        api "/admin/v1/email/smtp/$id" --request PUT --data "$want" >/dev/null
      fi
      password | api "/admin/v1/email/smtp/$id/password" --request PUT --data @- >/dev/null
    fi
    if [ "$(jq --raw-output '.state // empty' <<<"$have")" != EMAIL_PROVIDER_ACTIVE ]; then
      api "/admin/v1/email/$id/_activate" --data '{}' >/dev/null
    fi
  '';

  # an update sets all of a policy's fields, so each is sent back as read with only the three check lifetimes changed,
  # and only when one differs: ZITADEL refuses an update that changes nothing
  setSignInChecks = ''
    lifetime=${toString (cfg.signIn.checkDays * 86400)}s
    keep() {
      local have
      have=$(api "$1" "''${@:2}")
      if [ "$(jq --arg l "$lifetime" \
        '.policy | [.passwordCheckLifetime, .secondFactorCheckLifetime, .multiFactorCheckLifetime] == [$l, $l, $l]' \
        <<<"$have")" != true ]; then
        jq --arg l "$lifetime" '.policy
          | {allowUsernamePassword, allowRegister, allowExternalIdp, forceMfa, passwordlessType, hidePasswordReset,
            ignoreUnknownUsernames, defaultRedirectUri, passwordCheckLifetime, externalLoginCheckLifetime,
            mfaInitSkipLifetime, secondFactorCheckLifetime, multiFactorCheckLifetime, allowDomainDiscovery,
            disableLoginWithEmail, disableLoginWithPhone, forceMfaLocalOnly}
          + {passwordCheckLifetime: $l, secondFactorCheckLifetime: $l, multiFactorCheckLifetime: $l}' <<<"$have" \
          | api "$1" "''${@:2}" --request PUT --data @- >/dev/null
      fi
    }

    keep /admin/v1/policies/login
    # an org without a policy of its own follows the instance's
    for org in $(api /admin/v1/orgs/_search --data '{"query": {"limit": 1000}}' | jq --raw-output '.result[]?.id'); do
      if [ "$(api /management/v1/policies/login --header "x-zitadel-orgid: $org" | jq '.isDefault // false')" != true ]; then
        keep /management/v1/policies/login --header "x-zitadel-orgid: $org"
      fi
    done
  '';

  # one role over the socket with peer auth: Admin = User, so init's admin steps skip themselves
  role = "zitadel";
  database = "zitadel";
  socketDir = "/run/postgresql";
  postgres = config.services.postgresql.package;
in
{
  options.bundles.zitadel = {
    enable = lib.mkEnableOption "ZITADEL at ${hostname}";
    mail.enable = lib.mkEnableOption ''
      ZITADEL's mail through Stalwart's submission listener as zitadel@, its provider set again at
      each start through the admin API with an IAM owner's token. Reading that mail takes a ZITADEL
      sign-in, so a lost sign-in can't be reset by mail; the way back in is in the private notes
      repo's homelab/zitadel-break-glass.md
    '';
    signIn.checkDays = lib.mkOption {
      type = lib.types.nullOr lib.types.ints.positive;
      default = null;
      description = ''
        Days a password, second factor or passkey check counts before ZITADEL asks for it again, on the instance's
        login policy and on each org that keeps its own, set again at each start through the admin API with an IAM
        owner's token. null leaves ZITADEL's own (10 days, 18 h, 12 h).
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.systemd.services.zitadel.serviceConfig ? LoadCredential;
        message = ''
          bundles.zitadel keeps its secrets root-owned and needs services.zitadel from
          modules/zitadel, which passes them in as credentials. Import it and add
          "services/web-apps/zitadel.nix" to disabledModules.
        '';
      }
    ];

    sops.secrets = zitadel-secrets.secrets;

    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_18;
      ensureDatabases = [ database ];
      ensureUsers = [
        {
          name = role;
          ensureDBOwnership = true;
        }
      ];
    };

    services.zitadel = {
      enable = true;
      package = pkgs.callPackage ../packages/zitadel { };
      user = role;
      openFirewall = false;

      masterKeyFile = get-zitadel-secret "master_key";
      extraStepsPaths = [ (get-zitadel-secret "admin_steps") ];
      extraSettingsPaths = [ (get-zitadel-secret "settings") ];

      tlsMode = "external";
      settings = {
        Port = port;
        ExternalPort = 443;
        ExternalDomain = hostname;
        Database.postgres =
          let
            connection = {
              Username = role;
              Password = "";
              SSL.Mode = "disable";
            };
          in
          {
            Host = socketDir;
            Port = config.services.postgresql.settings.port;
            Database = database;
            MaxOpenConns = 15;
            MaxIdleConns = 10;
            MaxConnLifetime = "1h";
            MaxConnIdleTime = "5m";
            User = connection;
            Admin = connection // {
              ExistingDatabase = database;
            };
          };
        # the default asks for an RFC1918 address, the GCE metadata server, or panics
        Machine.Identification = {
          PrivateIp.Enabled = false;
          Hostname.Enabled = true;
          Webhook.Enabled = false;
        };
        ServicePing.Enabled = false;
      };
    };

    systemd.services.zitadel = {
      requires = [ "postgresql.target" ];
      after = [ "postgresql.target" ];
    };

    systemd.services.zitadel-mail = lib.mkIf cfg.mail.enable (adminUnit {
      name = "zitadel-mail";
      description = "ZITADEL's mail provider, Stalwart's submission listener";
      credentials = {
        password = get-zitadel-secret "smtp_password";
        inherit token;
      };
      text = setMailProvider;
    });

    systemd.services.zitadel-sign-in = lib.mkIf (cfg.signIn.checkDays != null) (adminUnit {
      name = "zitadel-sign-in";
      description = "How long ZITADEL's sign-in checks count, on the instance's login policy and each org's own";
      credentials = { inherit token; };
      text = setSignInChecks;
    });

    bundles.backup.services.zitadel.dumpCommand = pkgs.writeShellScript "zitadel-dump" ''
      exec ${pkgs.util-linux}/bin/setpriv --reuid=${role} --regid=${role} --clear-groups \
        ${postgres}/bin/pg_dump --format=custom --host=${socketDir} ${database}
    '';

    networking.interfaces.enp1s0.ipv6.addresses = [
      {
        address = zitadelAddr;
        prefixLength = ips.ipv6.prefixLength;
      }
    ];

    bundles.reverse-proxy = {
      enable = true;
      openFirewall = true;
      acme-email = "me@ironmoon.dev";
      hosts.${hostname} = {
        inherit port;
        websockets = true;
        listenAddresses = [
          ips.ipv4.address
          zitadelAddr
        ];
      };
    };
  };
}
