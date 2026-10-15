{ config, lib, ... }:
{
  # Stage 2 has no networkd to take over the initrd's state, which systemd's
  # network_is_online() then reads as the host's for the whole boot: a root
  # switched before the link has carrier leaves it `offline`, and timesyncd
  # never syncs.
  boot.initrd.systemd.services.forget-initrd-network =
    lib.mkIf (config.boot.initrd.systemd.network.enable && !config.systemd.network.enable)
      {
        description = "Forget initrd networkd's state";
        after = [ "initrd-switch-root.target" ];
        before = [ "initrd-switch-root.service" ];
        wantedBy = [ "initrd-switch-root.service" ];
        unitConfig.DefaultDependencies = false;
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "/bin/rm -rf /run/systemd/netif";
        };
      };
}
