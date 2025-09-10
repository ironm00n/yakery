{
  config,
  lib,
  pkgs,
  ...
}:
{
  imports = [
    ./networked.nix
    ../../users/home-manager.nix
    ./specializations/hyprland.nix
    ./specializations/kde.nix
    ../../users/ironmoon/user.nix
  ];

  bundles.fonts.enable = true;
  bundles.mullvad-vpn.enable = false;
  bundles.nvidia.enable = config.host.nvidia;

  # SECURITY: this is fine for single user, personal systems.
  # TODO: make a specific group for this, it shouldn't just be wheel
  nix.settings.trusted-users = [
    "root"
    "@wheel"
  ];

  # use latest kernel
  boot.kernelPackages = pkgs.linuxPackages_latest;

  # bluetooth
  hardware.bluetooth = {
    enable = true;
    powerOnBoot = true;
  };

  # audios
  services.pulseaudio.enable = lib.mkDefault false;
  security.rtkit.enable = true;
  services.pipewire = {
    enable = lib.mkDefault true;
    alsa.enable = lib.mkDefault true;
    alsa.support32Bit = lib.mkDefault true;
    pulse.enable = lib.mkDefault true;
  };

  # Configure keymap in X11
  services.xserver.xkb = {
    layout = "us";
    variant = "";
  };

  # system wide environment variables
  environment.variables = {
    DO_NOT_TRACK = 1;
  };

  # zsh
  environment.pathsToLink = [
    "/share/zsh"
    "/share/xdg-desktop-portal"
    "/share/applications"
  ];
  users.defaultUserShell = pkgs.zsh;

  # services
  services.openssh.enable = true;
  services.udisks2.enable = true; # for calibre

  programs = {
    firefox = import ./programs/firefox.nix;
    thunderbird = import ./programs/thunderbird.nix;
    zsh = {
      enable = true;
      enableCompletion = false; # this interacts poorly with ~/.zshrc
    };
    gnupg.agent = {
      enable = true;
      enableSSHSupport = true;
    };
    partition-manager.enable = true;
    gnome-disks.enable = true;
    # FIXME: reenable when fixed
    ladybird.enable = false;
    dconf.enable = true;
    wireshark = {
      enable = true;
      package = pkgs.wireshark;
    };
  };

  # generate man pages
  # documentation.dev.enable = true;
  # documentation.man.generateCaches = true;

  environment.systemPackages = import ./pkgs/interactive.nix { inherit pkgs; };
}
