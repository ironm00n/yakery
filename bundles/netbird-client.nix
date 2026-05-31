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
    types
    optional
    ;
  cfg = config.bundles.netbird-client;

  netbird = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.netbird-deploy;
    prefix = "netbird";
  } [
    "external_server_setup_key"
  ];
in
{
  options.bundles.netbird-client = {
    enable = mkEnableOption "NetBird peer enrolled via a deployment key";

    managementUrl = mkOption {
      type = types.str;
      default = "https://vpn.ironmoon.dev:443";
      description = "Self-hosted management server the peer registers against.";
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

  config = mkIf cfg.enable {
    sops.secrets = netbird.secrets;

    services.netbird.clients.netbird = {
      port = 51820;

      login.enable = true;
      login.setupKeyFile = netbird.get-path "external_server_setup_key";
      # phantom unit under the activation-script path, so gate on it existing
      login.systemdDependencies = optional config.sops.useSystemdActivation "sops-install-secrets.service";

      environment.NB_MANAGEMENT_URL = cfg.managementUrl;
    };

    networking.firewall.interfaces.${config.services.netbird.clients.netbird.interface} = {
      inherit (cfg) allowedTCPPorts allowedUDPPorts;
    };
  };
}
