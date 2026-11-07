# The account Claude Code's background sessions run as when Owen starts them there (agents/work #66): no root, its own
# ssh key and user manager, memory caps none of its sessions can lift, and a tap of its own under a third uid, so the
# pool token its turns go out under is never the account's to read.
{
  config,
  options,
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
  tap = cfg.tap // {
    user = "${cfg.name}-tap";
    proxy = "127.0.0.1:${toString cfg.tap.port}";
  };
  # A loopback port reached by one uid alone: the account's tap by the account, the owner's own services by the owner.
  only =
    uid: port:
    "ip daddr 127.0.0.1 tcp dport ${toString port} meta skuid != ${toString uid} reject with tcp reset";
  portRule = ''
    chain output {
      type filter hook output priority filter; policy accept;
      ${lib.concatStringsSep "\n  " (
        [ (only cfg.uid tap.port) ] ++ map (only config.users.users.${cfg.owner}.uid) cfg.ownerPorts
      )}
    }
  '';
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

    ownerPorts = mkOption {
      type = types.listOf types.port;
      default = [ ];
      example = [
        8384
        8888
        8889
      ];
      description = "The owner's loopback services that answer as the owner, which only the owner's uid may reach.";
    };

    tap = {
      profile = mkOption {
        type = types.path;
        default = "/nix/var/nix/profiles/${cfg.name}-tap";
        description = ''
          A profile holding the interceptor, bin/anthropic-tap and share/anthropic-tap/policy.env, which the notes repo
          installs as Owen's own tap is installed; the unit waits for it.
        '';
      };

      port = mkOption {
        type = types.port;
        default = 8891;
      };

      poolTokenFile = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "The pool token the account's turns go out under; with none, its turns go to Anthropic as sent.";
      };

      watchers = mkOption {
        type = types.listOf types.str;
        default = [ cfg.owner ];
        description = "Accounts that may read the tap's socket, the account's conversation.";
      };
    };
  };

  config = mkIf cfg.enable (
    lib.mkMerge [
      # on a host with home-manager, the account's Claude Code settings are Owen's module as the agent's
      (lib.optionalAttrs (options ? home-manager) {
        home-manager.users.${cfg.name} = {
          imports = [ ../users/ironmoon/bundles/dev/claude ];
          bundles.dev.claude = {
            enable = true;
            account = "agent";
          };
          home.stateVersion = config.system.stateVersion;
        };
      })
      {
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
              assertion = cfg.ownerPorts == [ ] || config.users.users.${cfg.owner}.uid != null;
              message = "bundles.claude-agent: ownerPorts needs ${cfg.owner}'s uid fixed, for the port rule to name";
            }
            {
              assertion =
                !(lib.any (lib.flip lib.elem config.nix.settings.trusted-users) [
                  cfg.name
                  "@${cfg.name}"
                  "*"
                ]);
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

        # notes' claude-tap reads this to find the account's tap
        environment.etc."anthropic-tap/${cfg.name}".text = "${tap.user}.service";
        users.groups.${tap.user}.members = tap.watchers;
        users.users.${tap.user} = {
          isSystemUser = true;
          group = tap.user;
        };

        systemd.services.${tap.user} = {
          description = "anthropic-tap for ${cfg.name}'s sessions";
          wantedBy = [ "multi-user.target" ];
          after = [
            (if config.networking.nftables.enable then "nftables.service" else "${tap.user}-port.service")
          ];
          requires = lib.optional (!config.networking.nftables.enable) "${tap.user}-port.service";
          unitConfig.ConditionPathExists = "${tap.profile}/bin/anthropic-tap";
          environment = {
            STATE_DIR = "/var/lib/${tap.user}";
            TAP_SOCK = "/run/${tap.user}/events.sock";
            TAP_SOCK_MODE = "0660";
            LISTEN_ADDR = "off";
            LISTEN_PROXY = tap.proxy;
          }
          // lib.optionalAttrs (tap.poolTokenFile != null) { DISPATCH = "pool"; };
          serviceConfig = {
            ExecStart = "${tap.profile}/bin/anthropic-tap";
            EnvironmentFile = "${tap.profile}/share/anthropic-tap/policy.env";
            User = tap.user;
            Group = tap.user;
            StateDirectory = tap.user;
            StateDirectoryMode = "0755";
            RuntimeDirectory = tap.user;
            RuntimeDirectoryMode = "0750";
            LoadCredential = lib.optional (tap.poolTokenFile != null) "pool:${tap.poolTokenFile}";
            Restart = "on-failure";
          };
        };

        networking.nftables.tables.${tap.user} = mkIf config.networking.nftables.enable {
          family = "inet";
          content = portRule;
        };
        systemd.services."${tap.user}-port" = mkIf (!config.networking.nftables.enable) {
          description = "only ${cfg.name} reaches its tap's proxy port";
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = "${pkgs.nftables}/bin/nft -f ${pkgs.writeText "${tap.user}.nft" "table inet ${tap.user} {\n${portRule}}\n"}";
            ExecStop = "${pkgs.nftables}/bin/nft delete table inet ${tap.user}";
          };
        };

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
      }
    ]
  );
}
