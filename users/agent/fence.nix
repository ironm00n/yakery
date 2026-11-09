# The loopback fence: each port listed for him answers his uid alone, and each listed for the account answers it alone.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (import ./account.nix) name;
  cfg = config.agent.loopback;
  table = "${name}-loopback";
  only =
    uid: port:
    "ip daddr 127.0.0.1 tcp dport ${toString port} meta skuid != ${toString uid} reject with tcp reset";
  fence = ''
    chain output {
      type filter hook output priority filter; policy accept;
      ${lib.concatStringsSep "\n  " (
        map (only config.users.users.${name}.uid) cfg.own
        ++ map (only config.users.users.ironmoon.uid) cfg.his
      )}
    }
  '';
in
{
  options.agent.loopback = {
    his = lib.mkOption {
      type = with lib.types; listOf port;
      description = "Loopback ports of his own services, which no other uid reaches.";
    };
    own = lib.mkOption {
      type = with lib.types; listOf port;
      default = [ ];
      description = "Loopback ports of the account's own services, which no other uid reaches.";
    };
    unit = lib.mkOption {
      type = lib.types.str;
      readOnly = true;
      default = if config.networking.nftables.enable then "nftables.service" else "${table}.service";
      description = "The unit that puts the fence up, for a listener on a fenced port to start after.";
    };
  };

  config = {
    assertions = [
      {
        assertion = config.users.users.ironmoon.uid != null;
        message = "users/agent: the loopback fence needs ironmoon's uid fixed, to name it";
      }
    ];
    # Syncthing's GUI
    agent.loopback.his = [ 8384 ];

    networking.nftables.tables.${table} = lib.mkIf config.networking.nftables.enable {
      family = "inet";
      content = fence;
    };
    systemd.services.${table} = lib.mkIf (!config.networking.nftables.enable) {
      description = "his loopback services answer him alone, and the account's answer it alone";
      wantedBy = [ "multi-user.target" ];
      before = [ "network-pre.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${pkgs.nftables}/bin/nft -f ${pkgs.writeText "${table}.nft" "table inet ${table} {\n${fence}}\n"}";
        ExecStop = "${pkgs.nftables}/bin/nft delete table inet ${table}";
      };
    };
  };
}
