# Edit this configuration file to define what should be installed on
# your system. Help is available in the configuration.nix(5) man page, on
# https://search.nixos.org/options and in the NixOS manual (`nixos-help`).

{
  config,
  lib,
  pkgs,
  ...
}:

{
  imports = [
    ./hardware-configuration.nix
    ../common/interactive.nix
  ];

  # secure boot
  boot.bootspec.enable = true;
  boot.loader = {
    efi = {
      canTouchEfiVariables = true;
    };
    limine = {
      enable = true;
      secureBoot.enable = true;
    };
  };
  boot.initrd.systemd.enable = true; # TPM

  security.tpm2 = {
    enable = true;
    pkcs11.enable = true;
    tctiEnvironment.enable = true;
  };

  environment.etc."crypttab".text = ''
    cryptroot UUID=06c30faa-28d3-4881-8e82-d2089f2fd760 - tpm2-device=auto,tpm2-pcrs=0+1+7,discard
  '';

  # List packages installed in system profile.
  # You can use https://search.nixos.org/ to find more packages (and options).
  environment.systemPackages = with pkgs; [
    tpm2-tss
    tpm2-tools
    cryptsetup
    sbctl
  ];

  services.fwupd.enable = true;

  boot.initrd.kernelModules = [ "pinctrl_tigerlake" ];
  boot.kernelModules = [ "soc_button_array" ];

  # This option defines the first version of NixOS you have installed on this particular machine,
  # and is used to maintain compatibility with application data (e.g. databases) created on older NixOS versions.
  #
  # Most users should NEVER change this value after the initial install, for any reason,
  # even if you've upgraded your system to a new NixOS release.
  #
  # This value does NOT affect the Nixpkgs version your packages and OS are pulled from,
  # so changing it will NOT upgrade your system - see https://nixos.org/manual/nixos/stable/#sec-upgrading for how
  # to actually do that.
  #
  # This value being lower than the current NixOS release does NOT mean your system is
  # out of date, out of support, or vulnerable.
  #
  # Do NOT change this value unless you have manually inspected all the changes it would make to your configuration,
  # and migrated your data accordingly.
  #
  # For more information, see `man configuration.nix` or https://nixos.org/manual/nixos/stable/options#opt-system.stateVersion .
  system.stateVersion = "25.05"; # Did you read the comment?

}
