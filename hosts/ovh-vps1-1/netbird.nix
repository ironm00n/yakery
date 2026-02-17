{
  config,
  inputs,
  lib,
  ...
}:
# based on https://lukadeka.com/blog/setting-up-netbird-with-zitadel-on-nixos/
let
  domain = "ironmoon.dev";
  netbirdDomain = "vpn.${domain}";
  clientId = "360244641284554753";

  sopsFile = inputs.secrets.lib.netbird;
  mkNetbirdSecrets =
    secret:
    secret
    |> builtins.map (
      {
        key,
        usergroup ? null,
        owner ? usergroup,
        group ? usergroup,
      }:
      {
        name = "netbird_${key}";
        value = {
          inherit sopsFile;
          inherit key owner group;
        };
      }
    )
    |> builtins.listToAttrs;
  secret = key: config.sops.secrets.${"netbird_" + key}.path;
in
{
  sops.age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];
  sops.secrets = mkNetbirdSecrets [
    {
      key = "turn_password";
      usergroup = "turnserver";
    }
    { key = "data_store_encryption_key"; }
    { key = "relay_secret_container"; }
    { key = "relay_secret"; }
    { key = "setup_env"; }
  ];

  services.netbird.server = {
    enable = true;
    enableNginx = true;
    domain = netbirdDomain;

    coturn = {
      enable = true;
      domain = netbirdDomain;
      passwordFile = secret "turn_password";
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
        Signal.URI = "${netbirdDomain}:443";

        HttpConfig.AuthAudience = clientId;
        IdpManagerConfig.ClientConfig.ClientID = clientId;
        DeviceAuthorizationFlow.ProviderConfig = {
          Audience = clientId;
          ClientID = clientId;
        };
        PKCEAuthorizationFlow.ProviderConfig = {
          Audience = clientId;
          ClientID = clientId;
        };

        TURNConfig = {
          Secret._secret = secret "turn_password";
          CredentialsTTL = "12h";
          TimeBasedCredentials = false;
          Turns = [
            {
              Password._secret = secret "turn_password";
              Proto = "udp";
              URI = "turn:${netbirdDomain}:3478";
              Username = "netbird";
            }
          ];
        };
        Relay = {
          Addresses = [ "rels://${netbirdDomain}:33080" ];
          CredentialsTTL = "24h";
          Secret._secret = secret "relay_secret";
        };
        DataStoreEncryptionKey._secret = secret "data_store_encryption_key";
      };
    };
  };

  # Make the env available to the systemd service
  systemd.services.netbird-management.serviceConfig = {
    EnvironmentFile = secret "setup_env";
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
      (secret "relay_secret_container")
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
