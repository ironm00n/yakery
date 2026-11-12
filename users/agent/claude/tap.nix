# Each account's own anthropic-tap, under a third uid, so the pool token its turns go out under is never the account's
# to read. The fence keeps its port to the account and his tap's ports to him.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  accounts = builtins.attrValues (import ../accounts.nix);
  account =
    { name, tapPort, ... }:
    let
      user = "${name}-claude-tap";
      # His to write, so installing the tap (an out-link) or its token needs no root; PID1 reads the token as root.
      # Each lands by one rename from `new` into `pending`, and the restart moves it into place. A path unit watches
      # nothing while its unit runs, so it watches a state, `pending` not empty, which it reads again once that ends.
      install = "/var/lib/${user}-install";
      pending = "${install}/pending";
      tap = "${install}/tap";
      token = "${install}/pool-token";
      installed = lib.concatMapStringsSep " | " baseNameOf [
        tap
        token
      ];
      # What it can't move in, or has no use for, goes aside, so `pending` empties and its path unit goes quiet. Exits 0
      # once it moved one in, 1 when it moved none, and 255 on any other failure: ExecCondition's skip and fail.
      moveIn = pkgs.writeShellScript "${user}-move-in" ''
        set -e
        trap 'exit 255' ERR
        shopt -s nullglob
        moved=1
        for f in ${pending}/*; do
          case ''${f##*/} in
            ${installed}) if mv -T "$f" ${install}/"''${f##*/}"; then moved=0 && continue; fi ;;
          esac
          aside=''${aside:-$(mktemp -d ${install}/refused.XXXXXX)}
          echo "''${f##*/} isn't moved in; it is kept in $aside" >&2
          mv "$f" "$aside"/
        done
        exit $moved
      '';
      systemctl = "${config.systemd.package}/bin/systemctl";
    in
    {
      agent.loopback.own.${name} = [ tapPort ];

      # notes' claude-tap reads this to find the account's tap
      environment.etc."anthropic-tap/${name}".text = "${user}.service";
      # the tap's socket is the account's conversation
      users.groups.${user}.members = [ "ironmoon" ];
      users.users.${user} = {
        isSystemUser = true;
        group = user;
      };

      systemd.tmpfiles.rules = [
        "d ${install} 0711 ironmoon - -"
        "d ${install}/new 0700 ironmoon - -"
        "d ${pending} 0700 ironmoon - -"
        # the out-link's own root is left behind in `new`
        "L /nix/var/nix/gcroots/${user} - - - - ${tap}"
      ];

      systemd.services.${user} = {
        description = "anthropic-tap for ${name}'s sessions";
        wantedBy = [ "multi-user.target" ];
        requires = [ config.agent.loopback.unit ];
        after = [ config.agent.loopback.unit ];
        unitConfig.ConditionPathExists = "${tap}/bin/anthropic-tap";
        environment = {
          STATE_DIR = "/var/lib/${user}";
          TAP_SOCK = "/run/${user}/events.sock";
          TAP_SOCK_MODE = "0660";
          LISTEN_ADDR = "off";
          LISTEN_PROXY = "127.0.0.1:${toString tapPort}";
          DISPATCH = "pool";
        };
        serviceConfig = {
          ExecStart = "${tap}/bin/anthropic-tap";
          EnvironmentFile = "${tap}/share/anthropic-tap/policy.env";
          User = user;
          Group = user;
          StateDirectory = user;
          StateDirectoryMode = "0755";
          RuntimeDirectory = user;
          RuntimeDirectoryMode = "0750";
          LoadCredential = [ "pool:${token}" ];
          Restart = "on-failure";
        };
      };

      # A path unit only starts its unit, a no-op while the tap runs, so it starts this one
      systemd.paths."${user}-install" = {
        wantedBy = [ "paths.target" ];
        pathConfig = {
          DirectoryNotEmpty = pending;
          Unit = "${user}-install.service";
        };
      };
      # The move is his, in his dir, as PID1's would follow a `pending` he swapped for a link; the restart is PID1's,
      # and only after a move in. An install is no crash, so it neither counts toward the tap's start limit nor meets
      # systemd's 5 here; if even the move aside fails, this unit's own limit ends the path unit.
      systemd.services."${user}-install" = {
        description = "restart ${user} on a new install";
        unitConfig = {
          StartLimitIntervalSec = 10;
          StartLimitBurst = 20;
        };
        serviceConfig = {
          Type = "oneshot";
          User = "ironmoon";
          ExecCondition = moveIn;
          ExecStart = [
            "+${systemctl} reset-failed ${user}.service"
            "+${systemctl} restart ${user}.service"
          ];
        };
      };
    };
in
{
  config = lib.mkMerge (
    [
      {
        # his own tap's two ports
        agent.loopback.his = [
          8888
          8889
        ];
        assertions = [
          {
            assertion = lib.allUnique (map (a: a.tapPort) accounts);
            message = "users/agent/accounts.nix: two accounts share a tap port";
          }
        ];
      }
    ]
    ++ map account accounts
  );
}
