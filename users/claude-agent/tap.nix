# The account's own anthropic-tap, under a third uid, so the pool token its turns go out under is never the account's
# to read; and the loopback fence: the tap's port answers the account alone, and his own services answer him alone.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  agent = "claude-agent";
  ironmoon = config.users.users.ironmoon;
  user = "${agent}-tap";
  port = 8891;
  # Syncthing's GUI and his tap's two ports
  hisPorts = [
    8384
    8888
    8889
  ];
  # notes' `just agent-tap-install` puts the interceptor here, as his own tap is installed
  profile = "/nix/var/nix/profiles/${user}";
  nftables = config.networking.nftables.enable;
  only =
    uid: port:
    "ip daddr 127.0.0.1 tcp dport ${toString port} meta skuid != ${toString uid} reject with tcp reset";
  fence = ''
    chain output {
      type filter hook output priority filter; policy accept;
      ${lib.concatStringsSep "\n  " (
        [ (only config.users.users.${agent}.uid port) ] ++ map (only ironmoon.uid) hisPorts
      )}
    }
  '';
in
{
  assertions = [
    {
      assertion = ironmoon.uid != null;
      message = "users/claude-agent: the loopback fence needs ironmoon's uid fixed, to name it";
    }
  ];

  # notes' claude-tap reads this to find the account's tap
  environment.etc."anthropic-tap/${agent}".text = "${user}.service";
  # the tap's socket is the account's conversation
  users.groups.${user}.members = [ "ironmoon" ];
  users.users.${user} = {
    isSystemUser = true;
    group = user;
  };

  systemd.services.${user} = {
    description = "anthropic-tap for ${agent}'s sessions";
    wantedBy = [ "multi-user.target" ];
    after = [ (if nftables then "nftables.service" else "${user}-port.service") ];
    requires = lib.optional (!nftables) "${user}-port.service";
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
      LoadCredential = [ "pool:/etc/${agent}/pool-token" ];
      Restart = "on-failure";
    };
  };

  networking.nftables.tables.${user} = lib.mkIf nftables {
    family = "inet";
    content = fence;
  };
  systemd.services."${user}-port" = lib.mkIf (!nftables) {
    description = "only ${agent} reaches its tap's proxy port";
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${pkgs.nftables}/bin/nft -f ${pkgs.writeText "${user}.nft" "table inet ${user} {\n${fence}}\n"}";
      ExecStop = "${pkgs.nftables}/bin/nft delete table inet ${user}";
    };
  };
}
