{ ... }:
{
  imports = [
    ./hyprland.nix
    ./kde.nix
    ./nvidia.nix
    ./fonts.nix
    ./mullvad-vpn.nix
    ./ctf.nix
    ./syssec.nix
    ./printing.nix
    ./virtualisation.nix
    ./gaming.nix
    ./reverse-proxy.nix
    ./local-tls.nix
    ./netbird-client.nix

    ./displaylink.nix
  ];
}
