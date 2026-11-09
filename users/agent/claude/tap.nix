# Each account's own anthropic-tap, under a third uid, so the pool token its turns go out under is never the account's
# to read. The fence keeps its port to the account and his tap's ports to him.
{ config, lib, ... }:
let
  accounts = builtins.attrValues (import ../accounts.nix);
  account =
    { name, tapPort, ... }:
    let
      user = "${name}-claude-tap";
      # His to write, so installing the tap (an out-link) or its token needs no root; PID1 reads the token as root.
      # Each lands by one rename from `new`: a path unit drops events while its unit runs, so the first of several
      # would restart the tap before the last.
      install = "/var/lib/${user}-install";
      tap = "${install}/tap";
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
          LoadCredential = [ "pool:${install}/pool-token" ];
          Restart = "on-failure";
        };
      };

      # A path unit only starts its unit, a no-op while the tap runs. It watches the directory, as a watch on the link
      # follows it to the old store path.
      systemd.paths."${user}-install" = {
        wantedBy = [ "paths.target" ];
        pathConfig = {
          PathChanged = install;
          Unit = "${user}-install.service";
        };
      };
      systemd.services."${user}-install" = {
        description = "restart ${user} on a new install";
        serviceConfig.Type = "oneshot";
        script = "systemctl restart ${user}.service";
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
