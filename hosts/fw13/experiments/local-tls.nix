{ ... }:
{
  bundles.local-tls = {
    enable = true;
    domains = [
      "bulwark.test"
      "stalwart.test"
    ];
  };

  bundles.reverse-proxy = {
    enable = true;
    acme-email = "me@ironmoon.dev";
    # TODO(prod)
    openFirewall = false;
    hosts = {
      "bulwark.test".port = 3000;
      "stalwart.test" = {
        port = 8080;
        # JMAP push is Server-Sent Events; don't let nginx buffer it.
        extraLocationConfig = "proxy_buffering off;";
      };
    };
  };
}
