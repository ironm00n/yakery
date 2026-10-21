{ pkgs }:
let
  inherit (pkgs) lib;

  # The desktop's shape: networkd in the initrd for its ssh unlock, none after
  # it. Its root switched before its link had carrier; here no DHCP server
  # answers on the vlan, so the link is never routable in the initrd either.
  client = {
    boot.initrd.systemd.enable = true;
    boot.initrd.network.enable = true;
    boot.initrd.systemd.network.networks."10-vlan" = {
      matchConfig.Name = "eth1";
      networkConfig.DHCP = "ipv4";
      linkConfig.RequiredForOnline = "routable";
    };
    networking.useDHCP = false;
    services.timesyncd = {
      enable = lib.mkForce true;
      servers = [ "ntp" ];
      fallbackServers = [ ];
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "initrd-network";

  nodes = {
    ntp = {
      services.chrony = {
        enable = true;
        servers = [ ];
        extraConfig = ''
          allow
          local stratum 10
        '';
      };
      networking.firewall.allowedUDPPorts = [ 123 ];
    };
    fixed = {
      imports = [
        client
        ../../hosts/common/base/initrd-network.nix
      ];
    };
    unfixed = client;
  };

  testScript = ''
    start_all()
    ntp.wait_for_unit("chronyd.service")
    for client in (fixed, unfixed):
        client.wait_for_unit("multi-user.target")

    with subtest("without the module, the initrd's state outlives it, not online"):
        unfixed.succeed("test -e /run/systemd/netif/state")
        unfixed.fail("grep -Eqx 'ONLINE_STATE=(online|partial)' /run/systemd/netif/state")

    with subtest("with it, stage 2 starts with no networkd state and timesyncd syncs"):
        fixed.fail("test -e /run/systemd/netif")
        fixed.wait_until_succeeds("timedatectl show -p NTPSynchronized --value | grep -qx yes", timeout=180)

    with subtest("meanwhile timesyncd without it never asks"):
        # a minute past fixed's sync spans two of timesyncd's shortest, 32 s polls
        unfixed.sleep(60)
        unfixed.succeed("systemctl is-active systemd-timesyncd.service")
        unfixed.succeed("timedatectl show -p NTPSynchronized --value | grep -qx no")
        clients = ntp.succeed("chronyc -n clients")
        for client, asked in ((fixed, True), (unfixed, False)):
            address = ntp.succeed(f"getent ahostsv4 {client.name} | head -1 | cut -d' ' -f1").strip()
            assert (address in clients.split()) == asked, f"{client.name} at {address}:\n{clients}"
  '';
}
