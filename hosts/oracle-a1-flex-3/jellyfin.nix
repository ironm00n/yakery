{ ... }:
let
  jellyfinPort = 8096;
in
{
  services.jellyfin.enable = true;

  bundles.netbird-client.allowedTCPPorts = [
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
