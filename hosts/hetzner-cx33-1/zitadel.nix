{
  config,
  lib,
  inputs,
  ...
}:
let
  domain = "ironmoon.dev";
  sopsFile = inputs.secrets.lib.zitadel;
in
{
  sops.age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];
  sops.secrets.zitadel-master-key = {
    inherit sopsFile;
    key = "master_key";
    owner = "zitadel";
    group = "zitadel";
  };
  sops.secrets.zitadel-admin-steps = {
    inherit sopsFile;
    key = "admin_steps";
    owner = "zitadel";
    group = "zitadel";
  };
  sops.secrets.zitadel-settings = {
    inherit sopsFile;
    key = "settings";
    owner = "zitadel";
    group = "zitadel";
  };
  sops.secrets.zitadel-postgres-env = {
    inherit sopsFile;
    key = "postgres_env";
  };

  users.users.zitadel = {
    isSystemUser = true;
    group = "zitadel";
  };
  users.groups.zitadel = { };

  # setup inspired by https://lukadeka.com/blog/setting-up-netbird-with-zitadel-on-nixos/
  services.zitadel = {
    enable = true;
    openFirewall = true;

    user = "zitadel";
    group = "zitadel";

    masterKeyFile = config.sops.secrets.zitadel-master-key.path;
    extraStepsPaths = [ config.sops.secrets.zitadel-admin-steps.path ];
    extraSettingsPaths = [ config.sops.secrets.zitadel-settings.path ];

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
    ports = [ "5432:5432" ];
    environmentFiles = [ config.sops.secrets.zitadel-postgres-env.path ];
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
