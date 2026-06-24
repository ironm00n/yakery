# Ethernet + Wi-Fi sometimes share one flat LAN -> ARP flux.
# See: https://www.kernel.org/doc/Documentation/networking/ip-sysctl.txt
{ ... }:
{
  boot.kernel.sysctl = {
    "net.ipv4.conf.all.arp_ignore" = 1;
    "net.ipv4.conf.all.arp_announce" = 2;
  };
}
