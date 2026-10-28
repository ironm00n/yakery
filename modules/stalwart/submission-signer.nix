{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.stalwart.submissionSigner;
  inherit (import ../../lib/stalwart.nix { inherit lib; }) reconcile fileSecret;
  package = pkgs.callPackage ../../crates/rust.nix { crate = "submission-signer"; };
  url = "http://127.0.0.1:${toString cfg.port}/";
  tokenDir = "/var/lib/stalwart-hook-token";
in
{
  options.services.stalwart.submissionSigner = {
    enable = lib.mkEnableOption ''
      the hook every mail an account submits goes through: a From or Sender that isn't the
      envelope sender's own address is refused, and a message with a recipient at one of
      {option}`services.stalwart.submissionSigner.domains` gets a `Submission-Signature`
      naming the account and the sender. While the hook can't answer, submissions fail
      with a temporary error
    '';

    seedFile = lib.mkOption {
      type = lib.types.path;
      description = "The ed25519 seed the header is signed with, 32 bytes in base64.";
    };

    domains = lib.mkOption {
      type = lib.types.nonEmptyListOf lib.types.str;
      description = "Domains whose recipients get the header.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8081;
      description = "Loopback port Stalwart reaches the hook on.";
    };
  };

  config = lib.mkIf config.services.stalwart.enable (
    lib.mkMerge [
      # declared even while off, so turning it off takes the hook out of Stalwart
      {
        services.stalwart.plan.sequence = lib.mkAfter [
          (reconcile "MtaHook" [ "url" ] (
            lib.optionalAttrs cfg.enable {
              submission-signer = {
                inherit url;
                enable = {
                  match = [ ];
                  "else" = "!is_empty(authenticated_as)";
                };
                stages = {
                  data = true;
                };
                tempFailOnError = true;
                httpAuth = {
                  "@type" = "Bearer";
                  bearerToken = fileSecret "/run/credentials/stalwart.service/submission_hook_token";
                };
              };
            }
          ))
        ];
      }
      (lib.mkIf cfg.enable {
        services.stalwart.credentials.submission_hook_token = "${tokenDir}/token";

        systemd.services.stalwart-hook-token = {
          description = "Token Stalwart presents to the submission signer";
          before = [
            "stalwart.service"
            "submission-signer.service"
          ];
          requiredBy = [
            "stalwart.service"
            "submission-signer.service"
          ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            StateDirectory = baseNameOf tokenDir;
            UMask = "0077";
          };
          script = ''
            cd ${tokenDir}
            if [ ! -s token ]; then
              head -c 32 /dev/urandom | base64 -w0 > token.new
              mv token.new token
            fi
          '';
        };

        systemd.sockets.submission-signer = {
          wantedBy = [ "sockets.target" ];
          listenStreams = [ "127.0.0.1:${toString cfg.port}" ];
        };
        systemd.services.stalwart = {
          wants = [ "submission-signer.socket" ];
          after = [ "submission-signer.socket" ];
        };

        systemd.services.submission-signer = {
          description = "Stalwart's hook for submitted mail";
          serviceConfig = {
            ExecStart = lib.concatStringsSep " " (
              [ "${lib.getExe package} serve --seed %d/seed --token %d/token" ]
              ++ map (domain: "--domain ${domain}") cfg.domains
            );
            LoadCredential = [
              "seed:${cfg.seedFile}"
              "token:${tokenDir}/token"
            ];
            DynamicUser = true;
            # it only answers on the socket systemd holds
            PrivateNetwork = true;
            ProtectSystem = "strict";
            ProtectHome = true;
            PrivateDevices = true;
            NoNewPrivileges = true;
            MemoryDenyWriteExecute = true;
            SystemCallFilter = [ "@system-service" ];
            Restart = "on-failure";
          };
        };
      })
    ]
  );
}
