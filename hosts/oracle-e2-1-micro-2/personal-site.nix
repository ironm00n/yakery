{ lib, pkgs, ... }:

let
  siteUser = "personal-site";
  siteGroup = "personal-site";

  # NOTE: manually cloned https://code.ironmoon.dev/ironmoon/personal-site
  workDir = "/var/www/personal-site";
  entry = "${workDir}/server/serve.js";

  nodejs = pkgs.nodejs_22;
in
{
  users.groups.${siteGroup} = { };

  users.users.${siteUser} = {
    isSystemUser = true;
    group = siteGroup;
    home = workDir;
    createHome = false;
  };

  systemd.services.personal-site = {
    description = "Personal Site";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      NODE_ENV = "production";
    };

    serviceConfig = {
      Type = "simple";
      User = siteUser;
      Group = siteGroup;

      WorkingDirectory = workDir;
      ExecStart = "${lib.getExe nodejs} ${entry}";

      Restart = "always";
      RestartSec = 2;

      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ReadWritePaths = [ workDir ];
    };
  };

  services.nginx = {
    enable = true;
    recommendedProxySettings = true;
    recommendedTlsSettings = true;

    virtualHosts."owen.foo" = {
      serverAliases = [ "owen.duckham.dev" ];

      enableACME = true;
      forceSSL = true;

      locations."/" = {
        proxyPass = "http://127.0.0.1:8080";
        proxyWebsockets = true;
        extraConfig = ''
          proxy_read_timeout 90s;
          proxy_connect_timeout 90s;
          proxy_send_timeout 90s;
        '';
      };
    };
  };

  security.acme = {
    acceptTerms = true;
    defaults.email = "owen@duckham.dev";
  };

  networking.firewall.allowedTCPPorts = [
    80
    443
  ];
}
