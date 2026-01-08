{ pkgs, lib, ... }:
let
  haste-server = pkgs.callPackage ../../packages/haste-server/package.nix { };

  haste-config = {
    host = "0.0.0.0";
    port = 8080;
    keyLength = 10;
    maxLength = 400000;
    staticMaxAge = 86400;
    recompressStaticAssets = false;
    logging = [
      {
        level = "verbose";
        type = "Console";
        colorize = true;
      }
    ];
    keyGenerator.type = "phonetic";
    rateLimits.categories.normal = {
      totalRequests = 500;
      every = 60000;
    };
    storage = {
      type = "postgres";
      connectionUrl = "postgres://postgres@127.0.0.1:5432/haste";
    };

    documents = {
      about = "${haste-server}/share/haste-server/about.md";
    };
    twitter = false;
  };

  format = pkgs.formats.json { };
in
{
  systemd.services.haste-server = {
    wantedBy = [ "multi-user.target" ];
    requires = [ "network.target" ];
    after = [ "network.target" ];

    serviceConfig = {
      User = "haste-server";
      DynamicUser = true;
      StateDirectory = "haste-server";
      WorkingDirectory = "/var/lib/haste-server";
      ExecStart = "${haste-server}/bin/haste-server ${format.generate "config.json" haste-config}";
    };

    path = with pkgs; [
      pkg
      coreutils
    ];
  };

  services.postgresql = {
    enable = true;
    package = pkgs.postgresql_16;

    settings = {
      listen_addresses = lib.mkForce "127.0.0.1";
      port = 5432;
    };

    ensureDatabases = [ "haste" ];

    authentication = lib.mkOverride 10 ''
      # TYPE  DATABASE  USER      ADDRESS         METHOD
      local   all       all                       trust
      host    all       all       127.0.0.1/32    trust
      host    all       all       ::1/128         trust
    '';
  };

  services.nginx = {
    enable = true;
    recommendedProxySettings = true;
    recommendedTlsSettings = true;

    virtualHosts."haste.tanzanite.dev" = {
      enableACME = true;
      forceSSL = true;

      locations."/" = {
        proxyPass = "http://127.0.0.1:8080";
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
