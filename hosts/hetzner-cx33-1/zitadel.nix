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
  networking.firewall.allowedTCPPorts = [
    80
    443
  ];

  # Ensure the mounted directory for the database exists
  system.activationScripts.makeZitadelDir = lib.stringAfter [ "var" ] ''
    mkdir -p /var/lib/zitadel-db
  '';

  services.nginx.enable = true;
  services.nginx.virtualHosts."auth.${domain}" = {
    forceSSL = true;
    enableACME = true;
    locations."/" = {
      proxyPass = "http://127.0.0.1:39995";
      proxyWebsockets = true;
      extraConfig = ''
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
      '';
    };
  };

  security.acme = {
    acceptTerms = true;
    defaults.email = "me@ironmoon.dev";
  };
}
