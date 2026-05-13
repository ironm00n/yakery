{
  config,
  inputs,
  my-lib,
  ...
}:
# based on https://lukadeka.com/blog/setting-up-netbird-with-zitadel-on-nixos/
let
  domain = "ironmoon.dev";
  netbirdDomain = "vpn.${domain}";
  clientId = "360244641284554753";

  netbird = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.netbird;
    prefix = "netbird";
  } [
    {
      key = "turn_password";
      usergroup = "turnserver";
    }
    "data_store_encryption_key"
    "relay_secret_container"
    "relay_secret"
    "setup_env"
    "idp_mgmt_client_secret"
  ];
  get-netbird-secret = netbird.get-path;
in
{
  sops.secrets = netbird.secrets;

  services.netbird.server = {
    enable = true;
    enableNginx = true;
    domain = netbirdDomain;

    coturn = {
      enable = true;
      domain = netbirdDomain;
      passwordFile = get-netbird-secret "turn_password";
    };

    signal = {
      enable = true;
      enableNginx = true;
      domain = netbirdDomain;
    };

    dashboard = {
      enable = true;
      enableNginx = true;
      domain = netbirdDomain;
      settings = {
        AUTH_AUTHORITY = "https://auth.${domain}";
        AUTH_CLIENT_ID = clientId;
        AUTH_AUDIENCE = clientId;
      };
    };

    management = {
      enable = true;
      enableNginx = true;
      domain = netbirdDomain;
      turnDomain = netbirdDomain;
      singleAccountModeDomain = netbirdDomain;
      oidcConfigEndpoint = "https://auth.${domain}/.well-known/openid-configuration";

      settings = {
        HttpConfig.AuthAudience = clientId;

        IdpManagerConfig = {
          ManagerType = "zitadel";

          ClientConfig = {
            ClientID = "netbird";
            ClientSecret._secret = get-netbird-secret "idp_mgmt_client_secret";
            GrantType = "client_credentials";
            TokenEndpoint = "https://auth.${domain}/oauth/v2/token";
          };

          ExtraConfig = {
            ManagementEndpoint = "https://auth.${domain}/management/v1";
          };
        };
        DeviceAuthorizationFlow = {
          Provider = "hosted";
          ProviderConfig = {
            Audience = clientId;
            ClientID = clientId;
          };
        };
        PKCEAuthorizationFlow.ProviderConfig = {
          Audience = clientId;
          ClientID = clientId;
          scope = "openid profile email offline_access api";
        };

        TURNConfig = {
          Secret._secret = get-netbird-secret "turn_password";
          CredentialsTTL = "12h";
          TimeBasedCredentials = false;
          Turns = [
            {
              Password._secret = get-netbird-secret "turn_password";
              Proto = "udp";
              URI = "turn:${netbirdDomain}:3478";
              Username = "netbird";
            }
          ];
        };
        Relay = {
          Addresses = [ "rels://${netbirdDomain}:33080" ];
          CredentialsTTL = "24h";
          Secret._secret = get-netbird-secret "relay_secret";
        };
        DataStoreEncryptionKey._secret = get-netbird-secret "data_store_encryption_key";
      };
    };
  };

  # Make the env available to the systemd service
  systemd.services.netbird-management.serviceConfig = {
    EnvironmentFile = get-netbird-secret "setup_env";
  };

  # Override ACME settings to get a cert
  services.nginx.virtualHosts = lib.mkMerge [
    {
      "${netbirdDomain}" = {
        enableACME = true;
        forceSSL = true;
      };
    }
  ];

  # Run the Netbird relay with TLS to allow relaying over TCP
  virtualisation.oci-containers.containers.netbird-relay = {
    image = "netbirdio/relay:latest";
    ports = [
      "33080:33080"
    ];
    volumes = [
      "/var/lib/acme/${netbirdDomain}/:/certs:ro"
    ];
    environment = {
      NB_LOG_LEVEL = "info";
      NB_LISTEN_ADDRESS = ":33080";
      NB_EXPOSED_ADDRESS = "rels://${netbirdDomain}:33080";
      NB_TLS_CERT_FILE = "/certs/fullchain.pem";
      NB_TLS_KEY_FILE = "/certs/key.pem";
    };
    environmentFiles = [
      (get-netbird-secret "relay_secret_container")
    ];
  };

  networking.firewall.allowedTCPPorts = [
    80
    443
    3478
    10000
    33080
  ];
  networking.firewall.allowedUDPPorts = [
    3478
    5349
    33080
  ];
  networking.firewall.allowedUDPPortRanges = [
    {
      from = 40000;
      to = 40050;
    }
  ];

  security.acme = {
    acceptTerms = true;
    defaults.email = "me@ironmoon.dev";
  };
}
