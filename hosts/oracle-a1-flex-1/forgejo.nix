{
  pkgs,
  lib,
  config,
  inputs,
  my-lib,
  ...
}:
let
  dbPort = 5432;
  clientId = "371976088601034753";
  conf = config.services.forgejo;

  forgejo-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.forgejo;
        prefix = "forgejo";
        separator = "-";
        usergroup = "forgejo";
      }
      [
        "client_secret"
      ];
  get-forgejo-secret = forgejo-secrets.get-path;
in
{
  sops.secrets = forgejo-secrets.secrets;

  services.forgejo = {
    enable = true;

    lfs.enable = true;
    settings = {
      DEFAULT.APP_NAME = "ironmoon's Forgejo";
      "ui.meta".AUTHOR = "ironmoon's Forgejo Instance";

      server = {
        DOMAIN = "code.ironmoon.dev";
        ROOT_URL = "https://code.ironmoon.dev";
        HTTP_ADDR = "127.0.0.1";
        HTTP_PORT = 3000;
        SSH_DOMAIN = "ssh.ironmoon.dev"; # code.ironmoon.dev is usually behind cloudflare.
        SSH_USER = "git";
        SSH_CREATE_AUTHORIZED_KEYS_FILE = false;
        SSH_AUTHORIZED_KEYS_COMMAND_TEMPLATE =
          "/run/wrappers/bin/sudo --preserve-env=SSH_ORIGINAL_COMMAND,SSH_CONNECTION,GIT_PROTOCOL "
          + "-u forgejo {{.AppPath}} --config={{.CustomConf}} serv key-{{.Key.ID}}";
      };

      service = {
        DISABLE_REGISTRATION = true;
        ALLOW_ONLY_EXTERNAL_REGISTRATION = true;
        ENABLE_BASIC_AUTHENTICATION = false;
        ENABLE_INTERNAL_SIGNIN = false;
      };

      admin = {
        EXTERNAL_USER_DISABLE_FEATURES = "manage_password";
      };

      oauth2_client = {
        ENABLE_AUTO_REGISTRATION = true;
        UPDATE_AVATAR = true;
        # TODO: https://codeberg.org/forgejo/forgejo/pulls/12504
        # USERNAME = "preferred_username";
        ACCOUNT_LINKING = "disabled";
      };

      "repository.pull-request" = {
        DEFAULT_MERGE_MESSAGE_ALL_AUTHORS = true;
      };

      repository = {
        PREFERRED_LICENSES = "AGPL-3.0-only,AGPL-3.0-or-later,GPL-3.0-only,GPL-3.0-or-later,GPL-2.0-or-later,GPL-2.0-only";
      };

      "git.timeout".MIGRATE = 21600;

      # TODO:
      actions = {
        ENABLED = false;
      };
    };

    database = {
      createDatabase = false;
      type = "postgres";
      socket = "/run/postgresql:${toString dbPort}";
    };
  };

  users.users.git = {
    isSystemUser = true;
    group = "git";
    shell = pkgs.bashInteractive;
  };
  users.groups.git = { };

  security.sudo.extraRules = [
    {
      users = [ "git" ];
      runAs = "forgejo";
      commands = [
        {
          command = "${lib.getExe conf.package} --config=* serv *";
          options = [
            "NOPASSWD"
            "SETENV"
          ];
        }
      ];
    }
  ];

  environment.etc."ssh/forgejo-authorized-keys" = {
    mode = "0755";
    text = ''
      #!${pkgs.runtimeShell}
      set -euo pipefail

      exec ${lib.getExe conf.package} \
        --config ${conf.customDir}/conf/app.ini \
        keys -e git -u "$1" -t "$2" -k "$3"
    '';
  };

  services.openssh.extraConfig = ''
    Match User git
      AuthorizedKeysCommand /etc/ssh/forgejo-authorized-keys %u %t %k
      AuthorizedKeysCommandUser forgejo
      AuthorizedKeysFile none
      AuthenticationMethods publickey
      PasswordAuthentication no
      KbdInteractiveAuthentication no
  '';

  systemd.services.forgejo-zitadel-oauth = {
    description = "Register Zitadel OIDC auth source in Forgejo";
    after = [ "forgejo.service" ];
    requires = [ "forgejo.service" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = my-lib.systemd.hardening.network // {
      Type = "oneshot";
      User = "forgejo";
      Group = "forgejo";
      RemainAfterExit = true;
      LoadCredential = [
        "client-secret:${get-forgejo-secret "client_secret"}"
      ];
    };

    path = [
      pkgs.gawk
      conf.package
    ];

    script = ''
      set -euo pipefail
      CSEC=$(cat "$CREDENTIALS_DIRECTORY/client-secret")

      FORGEJO="forgejo --config ${conf.customDir}/conf/app.ini"

      ID=$($FORGEJO admin auth list | awk -v n=zitadel '$2==n {print $1}')

      COMMON_ARGS=(
        --name zitadel
        --provider openidConnect
        --key ${clientId}
        --secret "$CSEC"
        --auto-discover-url "https://auth.ironmoon.dev/.well-known/openid-configuration"
        --scopes "openid profile email"
        --group-claim-name groups
        --required-claim-name groups
        --required-claim-value forgejo-user
        --admin-group forgejo-admin
        --skip-local-2fa
      )

      if [ -z "$ID" ]; then
        $FORGEJO admin auth add-oauth "''${COMMON_ARGS[@]}"
      else
        $FORGEJO admin auth update-oauth --id "$ID" "''${COMMON_ARGS[@]}"
      fi
    '';
  };

  systemd.services.forgejo.requires = [ "postgresql.target" ];

  bundles.reverse-proxy = {
    enable = true;
    openFirewall = true;
    acme-email = "me@ironmoon.dev";
    hosts."code.ironmoon.dev" = {
      port = conf.settings.server.HTTP_PORT;
      anubis = true;
      aliases = [ "c.irm.is" ];
      extraLocationConfig = ''
        client_max_body_size 512M;
        proxy_read_timeout 300s;
      '';
    };
  };
}
