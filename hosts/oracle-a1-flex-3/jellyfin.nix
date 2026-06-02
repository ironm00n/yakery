{ ... }:
let
  jellyfinPort = 8096;
in
{
  services.jellyfin.enable = true;

  bundles.vpn.netbird.allowedTCPPorts = [
    jellyfinPort
    80
  ];

  bundles.reverse-proxy = {
    enable = true;
    hosts."jellyfin.im.exposed" = {
      port = jellyfinPort;
      websockets = true;
      tls = false;
    };
  };
}
