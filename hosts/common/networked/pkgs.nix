{ pkgs }:

with pkgs;
[
  inetutils

  wget
  nmap
  dig
  netcat
  mtr

  # sets up server for remote development
  vscode-extensions.ms-vscode-remote.remote-ssh
  kitty.terminfo

  wireguard-tools
]
