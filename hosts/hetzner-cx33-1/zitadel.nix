{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  domain = "ironmoon.dev";
  zitadel-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.zitadel;
        prefix = "zitadel";
        separator = "-";
        owner = "zitadel";
        group = "zitadel";
      }
      [
        "master_key"
        "admin_steps"
        "settings"
        {
          key = "postgres_env";
          owner = null;
          group = null;
        }
      ];
  get-zitadel-secret = zitadel-secrets.get-path;
in
{
  sops.secrets = zitadel-secrets.secrets;

  users.users.zitadel = {
    isSystemUser = true;
    group = "zitadel";
  };
  users.groups.zitadel = { };

  # setup inspired by https://lukadeka.com/blog/setting-up-netbird-with-zitadel-on-nixos/
  services.zitadel = {
    enable = true;
    openFirewall = false;

    user = "zitadel";
    group = "zitadel";

    masterKeyFile = get-zitadel-secret "master_key";
    extraStepsPaths = [ (get-zitadel-secret "admin_steps") ];
    extraSettingsPaths = [ (get-zitadel-secret "settings") ];

    tlsMode = "external";
    settings = {
      Port = 39995;
      ExternalPort = 443;
      ExternalDomain = "auth.${domain}";
      Database = {
        postgres = {
          Host = "127.0.0.1";
          Port = 5432;
          Database = "zitadel";
          MaxOpenConns = 15;
          MaxIdleConns = 10;
          MaxConnLifetime = "1h";
          MaxConnIdleTime = "5m";
        };
      };
    };
  };
  virtualisation.oci-containers.containers.zitadel-db = {
    image = "postgres:17";
    ports = [ "127.0.0.1:5432:5432" ];
    environmentFiles = [ (get-zitadel-secret "postgres_env") ];
    volumes = [
      "/var/lib/zitadel-db:/var/lib/postgresql/data"
    ];
  };

  # Ensure the mounted directory for the database exists
  system.activationScripts.makeZitadelDir = lib.stringAfter [ "var" ] ''
    mkdir -p /var/lib/zitadel-db
  '';

  bundles.reverse-proxy = {
    enable = true;
    openFirewall = true;
    acme-email = "me@ironmoon.dev";
    hosts."auth.${domain}" = {
      port = 39995;
      websockets = true;
    };
  };
}
