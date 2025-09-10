{ config, pkgs, ... }:
{
  imports = [
    ./all.nix
  ];

  # Enable networking
  networking.networkmanager.enable = true;

  networking.hostName = config.host.hostname;

  environment.systemPackages = import ./pkgs/networked.nix { inherit pkgs; };
}
