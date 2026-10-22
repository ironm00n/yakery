{
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  agents = import ./agents.nix { inherit lib; };
  agent-mail = pkgs.callPackage ../../../crates/agent-mail/package.nix {
    deployment = my-lib.ironmoon;
    correspondents = [ ];
    correspondentsFor = { };
  };

  # the retired Python mailtool kept "user:" and "password:" lines here; it read the last line
  # whose key starts with "pass"
  mailtoolPassword = pkgs.writeShellScript "mailtool-password" ''
    ${lib.getExe pkgs.gawk} '{
      i = index($0, ":"); if (i == 0) next
      k = tolower(substr($0, 1, i - 1)); gsub(/^[ \t]+|[ \t]+$/, "", k)
      if (k ~ /^pass/) { v = substr($0, i + 1); gsub(/^[ \t\r]+|[ \t\r]+$/, "", v); p = v }
    } END { if (p != "") print p }' "$1"
  '';

  setup =
    name:
    let
      dir = "/home/${name}/.config/agent-mail";
      password = "${dir}/password";
      staged = "${password}.migrating";
      config = pkgs.writeText "agent-mail-${name}.json" (
        builtins.toJSON {
          login = "${name}@${my-lib.ironmoon.mail.agents.domain}";
          password-file = password;
        }
      );
    in
    pkgs.writeShellScript "agent-mail-setup-${name}" ''
      set -eu
      mkdir -p ${dir}
      if [ "$(readlink ${dir}/config.json)" != ${config} ]; then
        rm -rf ${dir}/config.json
        ln -s ${config} ${dir}/config.json
      fi
      [ -e ${password} ] && exit 0
      if [ -e /home/${name}/.zitadel_credentials ]; then
        ${mailtoolPassword} /home/${name}/.zitadel_credentials > ${staged}
      else
        echo passwordpassword > ${staged}
      fi
      if [ -s ${staged} ]; then
        mv ${staged} ${password}
      else
        rm -f ${staged}
        echo "no password in /home/${name}/.zitadel_credentials" >&2
        exit 1
      fi
    '';
in
{
  environment.systemPackages = [ agent-mail ];

  # Each agent sets up its own agent-mail as itself: nothing here runs as root in a home an agent
  # controls, and an agent that sabotages its setup loses only its own mail
  systemd.services = lib.mapAttrs' (
    name: _:
    lib.nameValuePair "agent-mail-setup-${name}" {
      description = "agent-mail config and password for ${name}";
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        User = name;
        Group = "agents";
        UMask = "0077";
        ExecStart = setup name;
        TimeoutStartSec = "30s";
        MemoryMax = "64M";
        MemorySwapMax = 0;
      };
    }
  ) agents;
}
