# The account itself (agents/work #66): no root, its own ssh key and user manager, and a memory cap nothing it runs can
# lift.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (import ./account.nix) name uid;
  home = config.users.users.${name}.home;
in
{
  assertions =
    map
      (group: {
        assertion =
          !(lib.elem group config.users.users.${name}.extraGroups)
          && !(lib.elem name (config.users.groups.${group}.members or [ ]));
        message = "users/agent: ${name} is in ${group}, which is root by another name";
      })
      [
        "wheel"
        "docker"
      ]
    ++ [
      {
        assertion =
          !(lib.any (lib.flip lib.elem config.nix.settings.trusted-users) [
            name
            "@${name}"
            "*"
          ]);
        message = "users/agent: ${name} is a nix trusted user, which is root by another name";
      }
    ];

  users.groups.${name}.gid = uid;
  users.users.${name} = {
    inherit uid;
    isNormalUser = true;
    group = name;
    linger = true;
  };

  # he starts, attaches to and moves its sessions
  security.sudo.extraRules = [
    {
      users = [ "ironmoon" ];
      runAs = name;
      commands = [
        {
          command = "ALL";
          options = [ "NOPASSWD" ];
        }
      ];
    }
  ];

  # everything the account runs, together
  systemd.slices."user-${toString uid}" = {
    overrideStrategy = "asDropin";
    sliceConfig.MemoryMax = "16G";
  };
  # made on the host it serves, so no other account's key is ever copied to it
  systemd.services."${name}-ssh-key" = {
    wantedBy = [ "multi-user.target" ];
    path = [ pkgs.openssh ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      User = name;
    };
    script = ''
      install -d -m 700 ${home}/.ssh
      [ -e ${home}/.ssh/id_ed25519 ] || ssh-keygen -q -t ed25519 -N "" -C "${name}@${config.networking.hostName}" -f ${home}/.ssh/id_ed25519
    '';
  };
}
