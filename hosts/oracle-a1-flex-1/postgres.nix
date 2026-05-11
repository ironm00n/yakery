{
  pkgs,
  lib,
  ...
}:
{
  services.postgresql = {
    enable = true;
    package = pkgs.postgresql_18;

    settings = {
      listen_addresses = lib.mkForce "127.0.0.1";
      port = 5432;
    };

    ensureDatabases = [ "forgejo" ];
    ensureUsers = [
      {
        name = "forgejo";
        ensureDBOwnership = true;
      }
      {
        name = "tanzanite";
      }
    ];

    authentication = ''
      # TYPE  DATABASE  USER       ADDRESS       METHOD
      local   forgejo   forgejo                  peer
      local   any       tanzanite                scram-sha-256
      host    any       tanzanite  127.0.0.1/32  scram-sha-256
      host    any       tanzanite  ::1/128       scram-sha-256
    '';
  };

  # NOTE:
  # - manually set tanzanite's password
  # - manually created databases: tanzanite, tanzanite-beta, tanzanite-dev, tanzanite-shared
}
