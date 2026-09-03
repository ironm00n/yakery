{
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  user = "artifacts";
  base = "/var/lib/${user}";
  port = 8932;

  # public A record for reads; a Netbird DNS override sends peers to the overlay address
  domain = "artifacts.ironmoon.dev";

  # gated on source address, not Host, which the client controls
  writeGuard = lib.concatLines (
    [
      "client_max_body_size 0;"
      ""
      "limit_except GET HEAD {"
    ]
    ++ map (cidr: "  allow ${cidr};") my-lib.netbird.overlayCidrs
    ++ [
      "  deny all;"
      "}"
    ]
  );

  # dufs lists /dir as readily as /dir/, so any dot-less final segment is overlay-only
  # (extensionless files aren't public either)
  listingGuard = lib.concatLines (
    [ "location ~ /[^/.]*$ {" ]
    ++ map (cidr: "  allow ${cidr};") my-lib.netbird.overlayCidrs
    ++ [
      "  deny all;"
      ""
      "  proxy_pass http://127.0.0.1:${toString port};"
      "  proxy_set_header Host $host;"
      "}"
    ]
  );

  # /v/<token>/foo serves foo, a cache buster for fetchers that ignore query strings;
  # must follow listingGuard since nginx takes the first matching regex location
  versionAlias = lib.concatLines [
    "location ~ ^/v/[^/]+/(?<vrest>.+)$ {"
    "  proxy_pass http://127.0.0.1:${toString port}/$vrest;"
    "  proxy_set_header Host $host;"
    ""
    "  limit_except GET HEAD {"
    "    deny all;"
    "  }"
    "}"
  ];
in
{
  users.groups.${user} = { };

  users.users.${user} = {
    isSystemUser = true;
    group = user;
    home = base;
  };

  systemd.services.artifacts = {
    description = "dufs static artifact host";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];

    serviceConfig = my-lib.systemd.hardening.network // {
      Type = "simple";
      User = user;
      Group = user;

      StateDirectory = user;
      WorkingDirectory = base;

      ExecStart = lib.concatStringsSep " " [
        (lib.getExe pkgs.dufs)
        "--bind 127.0.0.1"
        "--port ${toString port}"
        "--allow-upload"
        "--allow-delete"
        "--render-try-index"
        base
      ];

      Restart = "always";
      RestartSec = 2;

      # dufs enumerates interface addresses over netlink at startup
      RestrictAddressFamilies = my-lib.systemd.hardening.network.RestrictAddressFamilies ++ [
        "AF_NETLINK"
      ];
    };
  };

  bundles.reverse-proxy.hosts.${domain} = {
    inherit port;
    extraLocationConfig = writeGuard;
    extraVhostConfig = listingGuard + versionAlias;
  };
}
