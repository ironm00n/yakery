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
  domain = "ironmoon.dev";
  port = 39995;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  zitadelAddr = "${ips.ipv6.prefix}::1:1";
  zitadel-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.zitadel;
        prefix = "zitadel";
        separator = "-";
      }
      [
        "master_key"
        "admin_steps"
        "settings"
      ];
  get-zitadel-secret = zitadel-secrets.get-path;

  # one role over the socket with peer auth: Admin = User, so init's admin steps skip themselves
  role = "zitadel";
  database = "zitadel";
  socketDir = "/run/postgresql";
  postgres = config.services.postgresql.package;
in
{
  options.bundles.zitadel.enable = lib.mkEnableOption "ZITADEL at auth.${domain}";

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
        ExternalDomain = "auth.${domain}";
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
      hosts."auth.${domain}" = {
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
