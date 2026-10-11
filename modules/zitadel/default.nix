# NOTE: adapted from nixpkgs nixos/modules/services/web-apps/zitadel.nix at e158d9ed9b51
{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.services.zitadel;

  settingsFormat = pkgs.formats.yaml { };

  credentials =
    let
      numbered = kind: lib.imap0 (i: path: lib.nameValuePair "${kind}-${toString i}" path);
    in
    {
      settings = numbered "settings" cfg.extraSettingsPaths;
      steps = numbered "steps" cfg.extraStepsPaths;
      master-key = lib.nameValuePair "master-key" cfg.masterKeyFile;
    };
  inCredentialsDir = cred: "%d/${cred.name}";

  privateIpIdentification = cfg.settings.Machine.Identification.PrivateIp.Enabled or true;
in
{
  options.services.zitadel =
    let
      inherit (lib)
        mkEnableOption
        mkOption
        mkPackageOption
        types
        ;
    in
    {
      enable = mkEnableOption "ZITADEL, a user and identity access management platform";

      package = mkPackageOption pkgs "ZITADEL" { default = [ "zitadel" ]; };

      user = mkOption {
        type = types.str;
        default = "zitadel";
        description = "The user to run ZITADEL under.";
      };

      group = mkOption {
        type = types.str;
        default = "zitadel";
        description = "The group to run ZITADEL under.";
      };

      openFirewall = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Whether to open the port specified in `settings.Port` in the firewall.
        '';
      };

      masterKeyFile = mkOption {
        type = types.path;
        description = ''
          Path to a file containing a master encryption key for ZITADEL. The
          key must be 32 bytes.
        '';
      };

      tlsMode = mkOption {
        type = types.enum [
          "external"
          "enabled"
          "disabled"
        ];
        default = "external";
        example = "enabled";
        description = ''
          The TLS mode to use. Options are:

          - enabled: ZITADEL accepts HTTPS connections directly. You must
            configure TLS if this option is selected.
          - external: ZITADEL forces HTTPS connections, with TLS terminated at a
            reverse proxy.
          - disabled: ZITADEL accepts HTTP connections only. Should only be used
            for testing.
        '';
      };

      settings = mkOption {
        type = lib.types.submodule {
          freeformType = settingsFormat.type;

          options = {
            Port = mkOption {
              type = types.port;
              default = 8080;
              description = "The port that ZITADEL listens on.";
            };

            TLS = {
              KeyPath = mkOption {
                type = types.nullOr types.path;
                default = null;
                description = "Path to the TLS certificate private key.";
              };
              Key = mkOption {
                type = types.nullOr types.str;
                default = null;
                description = ''
                  The TLS certificate private key, as a base64-encoded string.

                  Note that the contents of this option will be added to the Nix
                  store as world-readable plain text. Set
                  [KeyPath](#opt-services.zitadel.settings.TLS.KeyPath) instead
                  if this is undesired.
                '';
              };
              CertPath = mkOption {
                type = types.nullOr types.path;
                default = null;
                description = "Path to the TLS certificate.";
              };
              Cert = mkOption {
                type = types.nullOr types.str;
                default = null;
                description = ''
                  The TLS certificate, as a base64-encoded string.

                  Note that the contents of this option will be added to the Nix
                  store as world-readable plain text. Set
                  [CertPath](#opt-services.zitadel.settings.TLS.CertPath) instead
                  if this is undesired.
                '';
              };
            };
          };
        };
        default = { };
        example = lib.literalExpression ''
          {
            Port = 8123;
            ExternalDomain = "example.com";
            TLS = {
              CertPath = "/path/to/cert.pem";
              KeyPath = "/path/to/cert.key";
            };
            Database.postgres.Host = "/run/postgresql";
          };
        '';
        description = ''
          Contents of the runtime configuration file. See
          <https://zitadel.com/docs/self-hosting/manage/configure> for more
          details.
        '';
      };

      extraSettingsPaths = mkOption {
        type = types.listOf types.path;
        default = [ ];
        description = ''
          A list of paths to extra settings files, read before
          [settings](#opt-services.zitadel.settings), so `settings` wins on
          keys set in both.
        '';
      };

      steps = mkOption {
        type = settingsFormat.type;
        default = { };
        example = lib.literalExpression ''
          {
            FirstInstance = {
              InstanceName = "Example";
              Org.Human = {
                UserName = "foobar";
                FirstName = "Foo";
                LastName = "Bar";
              };
            };
          }
        '';
        description = ''
          Contents of the database initialization config file. See
          <https://zitadel.com/docs/self-hosting/manage/configure> for more
          details.
        '';
      };

      extraStepsPaths = mkOption {
        type = types.listOf types.path;
        default = [ ];
        description = ''
          A list of paths to extra steps files, read before
          [steps](#opt-services.zitadel.steps).
        '';
      };
    };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          cfg.tlsMode == "enabled"
          -> (
            (cfg.settings.TLS.Key != null || cfg.settings.TLS.KeyPath != null)
            && (cfg.settings.TLS.Cert != null || cfg.settings.TLS.CertPath != null)
          );
        message = ''
          A TLS certificate and key must be configured in
          services.zitadel.settings.TLS if services.zitadel.tlsMode is enabled.
        '';
      }
    ];

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.settings.Port ];

    systemd.services.zitadel = {
      description = "ZITADEL identity access management";
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        Type = "exec";
        ExecStart =
          let
            configFile = settingsFormat.generate "config.yaml" cfg.settings;
            stepsFile = settingsFormat.generate "steps.yaml" cfg.steps;

            args = lib.cli.toCommandLineShellGNU { } {
              config = map inCredentialsDir credentials.settings ++ [ configFile ];
              steps = map inCredentialsDir credentials.steps ++ [ stepsFile ];
              masterkeyFile = inCredentialsDir credentials.master-key;
              inherit (cfg) tlsMode;
            };
          in
          "${lib.getExe cfg.package} start-from-init ${args}";
        LoadCredential = map (cred: "${cred.name}:${cred.value}") (
          credentials.settings ++ credentials.steps ++ [ credentials.master-key ]
        );
        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = "5s";

        AmbientCapabilities = "";
        CapabilityBoundingSet = "";
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateMounts = true;
        PrivateTmp = true;
        PrivateUsers = true;
        ProcSubset = "pid";
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectProc = "invisible";
        ProtectSystem = "strict";
        RemoveIPC = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ]
        ++ lib.optional privateIpIdentification "AF_NETLINK";
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
        ];
        UMask = "0077";
      };
    };

    users.users.zitadel = lib.mkIf (cfg.user == "zitadel") {
      isSystemUser = true;
      group = cfg.group;
    };
    users.groups.zitadel = lib.mkIf (cfg.group == "zitadel") { };
  };
}
