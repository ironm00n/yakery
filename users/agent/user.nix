# Each account itself (agents/work #66): no root, its own ssh key and user manager, and a memory cap nothing it runs can
# lift.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  accounts = builtins.attrValues (import ./accounts.nix);
  # every group a user is in, by its own entry or by a group's
  groupsOf =
    user:
    let
      entry = config.users.users.${user};
    in
    lib.unique (
      [ entry.group ]
      ++ entry.extraGroups
      ++ lib.attrNames (lib.filterAttrs (_: group: lib.elem user group.members) config.users.groups)
    );
  # his, and the two that are root by another name even once he leaves them
  barred = lib.subtractLists config.agent.groupsWithHim (
    groupsOf "ironmoon"
    ++ [
      "wheel"
      "docker"
    ]
  );
  account =
    {
      name,
      uid,
      secrets,
      ...
    }:
    let
      home = config.users.users.${name}.home;
    in
    {
      assertions = [
        (
          let
            shared = lib.intersectLists barred (groupsOf name);
          in
          {
            assertion = shared == [ ];
            message = "users/agent: ${name} is in ${lib.concatStringsSep ", " shared}, which no agent account may join (agent.groupsWithHim lists the groups it shares with him on purpose)";
          }
        )
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
      systemd.tmpfiles.rules = [ "d ${home}/${secrets} 0700 ${name} ${name} -" ];
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
    };
in
{
  options.agent.groupsWithHim = lib.mkOption {
    type = with lib.types; listOf str;
    default = [ ];
    description = "The groups of his that an agent account may join, each because sharing it is its point.";
  };

  config = lib.mkMerge (
    [
      {
        assertions = [
          {
            assertion = lib.allUnique (map (a: a.uid) accounts);
            message = "users/agent/accounts.nix: two accounts share a uid";
          }
        ];
      }
    ]
    ++ map account accounts
  );
}
