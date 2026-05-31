{ config, pkgs, ... }:
{
  imports = [
    ./all.nix
  ];

  # Enable networking
  networking.networkmanager.enable = true;

  networking.hostName = config.host.hostname;

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "prohibit-password";
    };
  };

  environment.systemPackages = import ./pkgs/networked.nix { inherit pkgs; };
}
