# Edit this configuration file to define what should be installed on
# your system.  Help is available in the configuration.nix(5) man page
# and in the NixOS manual (accessible by running ‘nixos-help’).

{ pkgs, ... }:
{
  imports = [
    ./hardware-configuration.nix
    ./disks.nix
    ./boot.nix
    ./restore.nix
    ../common/interactive
    ./networking.nix
  ];

  bundles = {
    displaylink.enable = true;
    gaming.enable = true;
    virtualisation = {
      enable = true;
      docker = true;
      waydroid = true;
    };
    vpn.mullvad.enable = true;
    vpn.globalprotect.enable = true;
  };

  services.fwupd.enable = true;

  services.minecraft-server = {
    enable = false;
    eula = true;
    openFirewall = true;
    package = pkgs.papermc;
    declarative = true;
    serverProperties = {
      difficulty = 3;
      motd = "ironmoon's server";
      max-players = 21;
      view-distance = 16;
      enable-command-block = true;
    };
  };

  # SSH
  # services.fail2ban.enable = true;
  services.openssh.settings = {
    AllowUsers = [
      "ironmoon"
      "root"
    ];
  };

  users.users.root.openssh.authorizedKeys.keys = import ../common/admin-ssh-keys.nix;

  # services.pulseaudio.enable = true;
  # security.rtkit.enable = true;
  # services.pipewire = {
  #   enable = false;
  #   alsa.enable = false;
  #   alsa.support32Bit = false;
  #   pulse.enable = false;
  # };

  # This value determines the NixOS release from which the default
  # settings for stateful data, like file locations and database versions
  # on your system were taken. It‘s perfectly fine and recommended to leave
  # this value at the release version of the first install of this system.
  # Before changing this value read the documentation for this option
  # (e.g. man configuration.nix or on https://nixos.org/nixos/options.html).
  system.stateVersion = "24.05"; # Did you read the comment?
}
