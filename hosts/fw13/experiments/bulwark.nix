{
  config,
  pkgs,
  my-lib,
  inputs,
  ...
}:
let
  client_id = "373816843720261633";
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
  sops.secrets = bulwark-secrets.secrets;
  disabledModules = [ "services/web-apps/bulwark.nix" ];
  imports = [../../../modules/bulwark];
  services.bulwark = {
    enable = true;
    package = (pkgs.callPackage ../../../packages/bulwark {});
    jmap_server_url = "https://stalwart.test";
    oauth = {
      enabled = true;
      only = true;
      client_id = client_id;
      client_secret_file = (get-bulwark-secret "oauth_client_secret");
      issuer_url = "https://auth.ironmoon.dev";
    };
    session_secret_file = (get-bulwark-secret "session_secret");
    settings_sync.enabled = true;
    telemetry.enabled = false;
    log.level = "debug";
  };

  systemd.services.bulwark.environment = {
    # Bulwark probes the JMAP URL server-side; Node must trust the local CA.
    NODE_EXTRA_CA_CERTS = config.bundles.local-tls.caCert;
    BULWARK_UPDATE_CHECK = "false";
    # Request offline_access so Zitadel actually mints a refresh token. The
    # "Refresh Token" grant type only *permits* it; the scope *requests* it.
    OAUTH_EXTRA_SCOPES = "offline_access";
  };
}
