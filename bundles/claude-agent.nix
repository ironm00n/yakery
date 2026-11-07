# The account Claude Code's background sessions run as when Owen starts them there (agents/work #66): no root, its own
# ssh key and user manager, and memory caps none of its sessions can lift.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.claude-agent;
  home = config.users.users.${cfg.name}.home;
in
{
  options.bundles.claude-agent = {
    enable = mkEnableOption "the claude-agent account for background Claude Code sessions";

    name = mkOption {
      type = types.str;
      default = "claude-agent";
    };

    uid = mkOption {
      type = types.int;
      default = 1500;
      description = "Fixed, so its user slice and user manager can be named in units.";
    };

    owner = mkOption {
      type = types.str;
      description = "The account that may run commands as this one, to start, attach to and move its sessions.";
    };

    memoryMax = mkOption {
      type = types.str;
      default = "16G";
      description = "Everything the account runs, together: its user slice's MemoryMax.";
    };

    sessionMemoryMax = mkOption {
      type = types.str;
      default = "2G";
      description = "Each session's own scope, which the sessions' spawn reads from CLAUDE_SESSION_MEMORY_MAX.";
    };

    authorizedKeys = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = "This account's keys on Owen's other hosts, so it logs in to itself there and never to him.";
    };
  };

  config = mkIf cfg.enable {
    assertions =
      map
        (group: {
          assertion =
            !(lib.elem group config.users.users.${cfg.name}.extraGroups)
            && !(lib.elem cfg.name (config.users.groups.${group}.members or [ ]));
          message = "bundles.claude-agent: ${cfg.name} is in ${group}, which is root by another name";
        })
        [
          "wheel"
          "docker"
        ]
      ++ [
        {
          assertion = !(lib.elem cfg.name config.nix.settings.trusted-users);
          message = "bundles.claude-agent: ${cfg.name} is a nix trusted user, which is root by another name";
        }
      ];

    users.groups.${cfg.name}.gid = cfg.uid;
    users.users.${cfg.name} = {
      inherit (cfg) uid;
      isNormalUser = true;
      group = cfg.name;
      linger = true;
      openssh.authorizedKeys.keys = cfg.authorizedKeys;
    };

    systemd.slices."user-${toString cfg.uid}" = {
      overrideStrategy = "asDropin";
      sliceConfig.MemoryMax = cfg.memoryMax;
    };
    systemd.services."user@${toString cfg.uid}" = {
      overrideStrategy = "asDropin";
      environment.CLAUDE_SESSION_MEMORY_MAX = cfg.sessionMemoryMax;
    };

    security.sudo.extraRules = [
      {
        users = [ cfg.owner ];
        runAs = cfg.name;
        commands = [
          {
            command = "ALL";
            options = [ "NOPASSWD" ];
          }
        ];
      }
    ];

    # made on the host it serves, so no other account's key is ever copied to it
    systemd.services."${cfg.name}-ssh-key" = {
      wantedBy = [ "multi-user.target" ];
      path = [ pkgs.openssh ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = cfg.name;
      };
      script = ''
        install -d -m 700 ${home}/.ssh
        [ -e ${home}/.ssh/id_ed25519 ] || ssh-keygen -q -t ed25519 -N "" -C "${cfg.name}@${config.networking.hostName}" -f ${home}/.ssh/id_ed25519
      '';
    };
  };
}
