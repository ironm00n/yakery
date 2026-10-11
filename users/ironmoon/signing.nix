{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  inherit (config.host) git-signing-key hostname;
  key = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.gpg.${hostname};
    prefix = "gpg";
    separator = "-";
    owner = "ironmoon";
  } [ "signing_key" ];
in
{
  config = lib.mkIf (git-signing-key != null) {
    assertions = [
      {
        assertion = builtins.pathExists ./resources/gpg/${git-signing-key}.asc;
        message = "host.git-signing-key: no users/ironmoon/resources/gpg/${git-signing-key}.asc";
      }
    ];

    sops.secrets = key.secrets;

    systemd.services.home-manager-ironmoon = lib.mkIf config.sops.useSystemdActivation {
      wants = [ "sops-install-secrets.service" ];
      after = [ "sops-install-secrets.service" ];
    };

    home-manager.users = lib.mkIf config.host.home-manager-nixos {
      ironmoon =
        { config, lib, ... }:
        {
          home.activation.importSigningKey = lib.hm.dag.entryAfter [ "importGpgKeys" ] ''
            run ${lib.getExe config.programs.gpg.package} --batch --quiet \
              --homedir ${lib.escapeShellArg config.programs.gpg.homedir} \
              --import ${key.get-path "signing_key"}
          '';
        };
    };
  };
}
