{ ... }:
{
  imports = [
    ./vpn
    ./sec.nix
    ./hyprland.nix
    ./kde.nix
    ./nvidia.nix
    ./fonts.nix
    ./printing.nix
    ./virtualisation.nix
    ./gaming.nix
    ./reverse-proxy.nix
    ./local-tls.nix
    ./distributed-builds.nix
    ./oom.nix
    ./appboxes.nix
    ./xdg-menu.nix

    ./displaylink.nix
  ];
}
