{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  inherit (lib)
    concatMapAttrs
    filterAttrs
    literalExpression
    mapAttrs
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.backup;
  outer = config;

  secretsOf =
    name: sopsFile:
    my-lib.sops.mkSecrets
      {
        config = outer;
        inherit sopsFile;
        prefix = "backup-${name}";
        separator = "-";
      }
      [
        "passphrase"
        "ssh-key"
      ];

  service =
    { name, config, ... }:
    {
      options = {
        dumpCommand = mkOption {
          type = types.path;
          description = "Program whose stdout is the backup, typically a database dump.";
        };

        startAt = mkOption {
          type = types.str;
          default = "daily";
          description = "systemd calendar expression the dump runs on.";
        };

        secretsFile = mkOption {
          type = types.nullOr types.path;
          default = inputs.secrets.lib.borg.${name};
          defaultText = literalExpression "inputs.secrets.lib.borg.\${name}";
          description = "sops file holding the repository `passphrase` and the `ssh-key` the target knows this service by.";
        };

        passphraseFile = mkOption {
          type = types.path;
          default = (secretsOf name config.secretsFile).get-path "passphrase";
          defaultText = "the `passphrase` in secretsFile";
          description = "File holding the repository passphrase.";
        };

        sshKeyFile = mkOption {
          type = types.path;
          default = (secretsOf name config.secretsFile).get-path "ssh-key";
          defaultText = "the `ssh-key` in secretsFile";
          description = "Private key the target's forced command is bound to.";
        };
      };
    };
in
{
  options.bundles.backup = {
    target = mkOption {
      type = types.submodule { options = my-lib.borg.targetOptions; };
      default = my-lib.borg.storagebox;
      description = "Host every repository lives on.";
    };

    services = mkOption {
      type = types.attrsOf (types.submodule service);
      default = { };
      description = ''
        Services backed up as the output of a dump command, each to its own repository on the
        target. The repository, key and passphrase belong to the service rather than the host, so
        declaring this next to the service carries the backup along when the service moves.

        The target is expected to bind each key to
        `borg serve --append-only --restrict-to-repository <home>/<name>`, so a compromised host
        can add archives but not destroy them; pruning is left to an unrestricted client.
      '';
    };
  };

  config = mkIf (cfg.services != { }) {
    sops.secrets = concatMapAttrs (name: svc: (secretsOf name svc.secretsFile).secrets) (
      filterAttrs (_: svc: svc.secretsFile != null) cfg.services
    );

    programs.ssh.knownHosts.backup-target = {
      hostNames = [ (my-lib.borg.knownHostName cfg.target) ];
      publicKey = cfg.target.hostPublicKey;
    };

    services.borgbackup.jobs = mapAttrs (name: svc: {
      repo = "ssh://${cfg.target.user}@${cfg.target.host}:${toString cfg.target.port}/./${name}";
      inherit (svc) dumpCommand startAt;
      encryption = {
        mode = "repokey-blake2";
        passCommand = "cat ${svc.passphraseFile}";
      };
      environment = {
        BORG_RSH = "ssh -i ${svc.sshKeyFile} -o IdentitiesOnly=yes -o BatchMode=yes";
        BORG_REMOTE_PATH = cfg.target.remotePath;
      };
      compression = "auto,zstd";
      extraCreateArgs = [
        "--stdin-name"
        "${name}.dump"
      ];
    }) cfg.services;
  };
}
