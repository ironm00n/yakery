{ config, pkgs, ... }:
{
  imports = [
    ./all.nix
  ];

  # Enable networking
  networking.networkmanager.enable = true;

  networking.hostName = config.host.hostname;

  programs.wireshark.enable = true;
  programs.wireshark.package = pkgs.wireshark;

  environment.systemPackages = import ./pkgs/networked.nix { inherit pkgs; };
}
