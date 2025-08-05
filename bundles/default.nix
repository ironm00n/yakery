{ ... }:
{
  imports = [
    ./hyprland.nix
    ./kde.nix
    ./nvidia.nix
    ./fonts.nix
    ./mullvad-vpn.nix
    ./ctf.nix
    ./printing.nix
    ./virtualisation.nix
    ./gaming.nix

    ./displaylink.nix
  ];
}
