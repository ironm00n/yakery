# The account's own anthropic-tap, under a third uid, so the pool token its turns go out under is never the account's
# to read. The fence keeps its port to the account and his tap's ports to him.
{ config, ... }:
let
  inherit (import ../account.nix) name;
  user = "${name}-claude-tap";
  port = 8891;
  # notes' `just agent-tap-install` puts the interceptor here, as his own tap is installed
  profile = "/nix/var/nix/profiles/${user}";
in
{
  agent.loopback = {
    own = [ port ];
    # his own tap's two ports
    his = [
      8888
      8889
    ];
  };

  # notes' claude-tap reads this to find the account's tap
  environment.etc."anthropic-tap/${name}".text = "${user}.service";
  # the tap's socket is the account's conversation
  users.groups.${user}.members = [ "ironmoon" ];
  users.users.${user} = {
    isSystemUser = true;
    group = user;
  };

  systemd.services.${user} = {
    description = "anthropic-tap for ${name}'s sessions";
    wantedBy = [ "multi-user.target" ];
    requires = [ config.agent.loopback.unit ];
    after = [ config.agent.loopback.unit ];
    unitConfig.ConditionPathExists = "${profile}/bin/anthropic-tap";
    environment = {
      STATE_DIR = "/var/lib/${user}";
      TAP_SOCK = "/run/${user}/events.sock";
      TAP_SOCK_MODE = "0660";
      LISTEN_ADDR = "off";
      LISTEN_PROXY = "127.0.0.1:${toString port}";
      DISPATCH = "pool";
    };
    serviceConfig = {
      ExecStart = "${profile}/bin/anthropic-tap";
      EnvironmentFile = "${profile}/share/anthropic-tap/policy.env";
      User = user;
      Group = user;
      StateDirectory = user;
      StateDirectoryMode = "0755";
      RuntimeDirectory = user;
      RuntimeDirectoryMode = "0750";
      LoadCredential = [ "pool:/etc/${user}/pool-token" ];
      Restart = "on-failure";
    };
  };
}
