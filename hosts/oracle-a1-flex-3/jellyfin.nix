{ ... }:
{
  services.jellyfin.enable = true;

  bundles.netbird-client.allowedTCPPorts = [ 8096 ];
}
