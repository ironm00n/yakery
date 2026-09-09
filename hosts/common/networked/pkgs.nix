{ pkgs }:

with pkgs;
[
  inetutils

  wget
  nmap
  dig
  netcat
  arp-scan

  wireguard-tools
] ++ [
  # todo: move this to different common layer and add unprivileged user too for more cushy headless boxes
  kitty.terminfo
] ++ [
  ethtool
  ndisc6
  tcpdump
  socat
  tio
  ngrep
  dhcpdump
  wavemon
]
