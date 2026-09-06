{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkOption
    mkIf
    mkMerge
    types
    optional
    ;
  cfg = config.bundles.vpn.netbird;

  netbird =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.netbird-deploy;
        prefix = "netbird";
      }
      [
        "external_server_setup_key"
      ];
in
{
  options.bundles.vpn.netbird = {
    enable = mkEnableOption "NetBird VPN client";

    useSetupKey = mkOption {
      type = types.bool;
      default = false;
      description = "Enroll headlessly with a deployment setup key (servers); otherwise interactive SSO login.";
    };

    managementUrl = mkOption {
      type = types.str;
      default = "https://vpn.ironmoon.dev:443";
      description = "Self-hosted management server the peer registers against.";
    };

    routing = mkOption {
      type = types.enum [
        "none"
        "client"
        "server"
        "both"
      ];
      default = "none";
      description = "`client` loosens reverse-path filtering to use an exit node; `server` enables forwarding to be one.";
    };

    allowedTCPPorts = mkOption {
      type = types.listOf types.port;
      default = [ ];
      description = "TCP ports to open on the netbird interface only.";
    };

    allowedUDPPorts = mkOption {
      type = types.listOf types.port;
      default = [ ];
      description = "UDP ports to open on the netbird interface only.";
    };
  };

  config = mkIf cfg.enable (mkMerge [
    {
      services.resolved.enable = true; # split dns

      services.netbird.useRoutingFeatures = cfg.routing;

      services.netbird.clients.netbird = {
        port = 51820;
        environment.NB_MANAGEMENT_URL = cfg.managementUrl;
      };

      networking.firewall.interfaces.${config.services.netbird.clients.netbird.interface} = {
        inherit (cfg) allowedTCPPorts allowedUDPPorts;
      };

      assertions = [
        {
          assertion = !config.services.netbird.enable;
          message = ''
            services.netbird.enable declares a second `default` client on wt0. Both
            resolve to netbird.service, `default` wins the name collision, and the
            per-interface rules above land on an nb-netbird that never comes up.
          '';
        }
      ];
    }

    (mkIf cfg.useSetupKey {
      sops.secrets = netbird.secrets;

      services.netbird.clients.netbird = {
        login.enable = true;
        login.setupKeyFile = netbird.get-path "external_server_setup_key";
        # phantom unit under the activation-script path, so gate on it existing
        login.systemdDependencies = optional config.sops.useSystemdActivation "sops-install-secrets.service";
      };
    })
  ]);
}
