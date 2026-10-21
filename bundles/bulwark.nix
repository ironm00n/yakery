{
  config,
  lib,
  pkgs,
  inputs,
  my-lib,
  ...
}:
let
  cfg = config.bundles.bulwark;
  inherit (my-lib.ironmoon) domain auth mail;
  hostname = "webmail.${domain}";
  port = 3000;
  ips = inputs.secrets.data.ips.${config.networking.hostName};
  address = "${ips.ipv6.prefix}::3";

  bulwark-secrets =
    my-lib.sops.mkSecrets
      {
        inherit config;
        sopsFile = inputs.secrets.lib.bulwark;
        prefix = "bulwark";
        separator = "-";
        usergroup = "bulwark";
      }
      [
        "session_secret"
        "oauth_client_secret"
      ];
  get-bulwark-secret = bulwark-secrets.get-path;
in
{
  imports = [ ../modules/bulwark ];
  disabledModules = [ "services/web-apps/bulwark.nix" ];

  options.bundles.bulwark.enable = lib.mkEnableOption "Bulwark webmail at ${hostname}";

  config = lib.mkIf cfg.enable {
    sops.secrets = bulwark-secrets.secrets;

    services.bulwark = {
      enable = true;
      package = pkgs.callPackage ../packages/bulwark { };
      inherit port;
      jmap_server_url = "https://${mail.host}";
      oauth = {
        enabled = true;
        only = true;
        client_id = "373816843720261633";
        client_secret_file = get-bulwark-secret "oauth_client_secret";
        issuer_url = "https://${auth.host}";
      };
      session_secret_file = get-bulwark-secret "session_secret";
      settings_sync.enabled = true;
      telemetry.enabled = false;
    };

    systemd.services.bulwark.environment = {
      BULWARK_UPDATE_CHECK = "false";
      # Zitadel only mints a refresh token when the scope asks for one
      OAUTH_EXTRA_SCOPES = "offline_access";
    };

    networking.interfaces.enp1s0.ipv6.addresses = [
      {
        inherit address;
        prefixLength = ips.ipv6.prefixLength;
      }
    ];

    bundles.reverse-proxy = {
      enable = true;
      openFirewall = true;
      acme-email = "me@${domain}";
      hosts.${hostname} = {
        inherit port;
        forwardedFor = "overwrite";
        listenAddresses = [
          ips.ipv4.address
          address
        ];
        listenOnOverlay = true;
        locations = {
          "/admin".netbirdOnly = true;
          "/api/admin".netbirdOnly = true;
        };
      };
    };
  };
}
