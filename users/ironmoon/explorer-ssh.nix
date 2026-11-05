{
  config,
  lib,
  inputs,
  my-lib,
  ...
}:
let
  key = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.explorer-ssh;
    prefix = "explorer-ssh";
    separator = "-";
    owner = "ironmoon";
  } [ "data" ];
in
{
  config = lib.mkIf config.host.explorer-ssh {
    sops.secrets = key.secrets;

    # Explorer takes only RC's key from 2026-11-06 09:00 and only the old one before; ssh tries them in order
    programs.ssh.extraConfig = ''
      Host login.explorer.northeastern.edu xfer.discovery.neu.edu
        User duckham.o
        IdentityFile ${key.get-path "data"}
        IdentityFile ~/.ssh/id_ed25519
        IdentitiesOnly yes
    '';
  };
}
