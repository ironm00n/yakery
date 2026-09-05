{
  pkgs,
  config,
  lib,
  ...
}:
{
  imports = [
    ../base
  ];

  # Enable networking
  networking.networkmanager.enable = true;

  networking.hostName = config.host.hostname;

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = lib.mkDefault "prohibit-password";
    };
  };

  programs = {
    mtr.enable = true;
  };

  environment.systemPackages = import ./pkgs.nix { inherit pkgs; };
}
