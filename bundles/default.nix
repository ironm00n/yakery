{ ... }:
{
  imports = [
    ./sec.nix
    ./hyprland.nix
    ./kde.nix
    ./nvidia.nix
    ./fonts.nix
    ./mullvad-vpn.nix
    ./printing.nix
    ./virtualisation.nix
    ./gaming.nix
    ./reverse-proxy.nix
    ./local-tls.nix
    ./netbird-client.nix
    ./distributed-builds.nix

    ./displaylink.nix
  ];
}
