{ lib, pkgs, my-lib, ... }:

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

    serviceConfig = my-lib.systemd.hardening.network // {
      Type = "simple";
      User = siteUser;
      Group = siteGroup;

      WorkingDirectory = workDir;
      ExecStart = "${lib.getExe nodejs} ${entry}";

      Restart = "always";
      RestartSec = 2;

      ReadWritePaths = [ workDir ];
      MemoryDenyWriteExecute = false; # V8 JIT
    };
  };

  bundles.reverse-proxy = {
    enable = true;
    openFirewall = true;
    acme-email = "owen@duckham.dev";
    hosts."owen.foo" = {
      port = 8080;
      aliases = [ "owen.duckham.dev" ];
      hsts.enable = true; # not strictly needed
    };
  };
}
