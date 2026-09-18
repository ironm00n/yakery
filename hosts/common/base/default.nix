{
  pkgs,
  config,
  lib,
  ...
}:
{
  # all bundles are behind an `enable` option
  imports = [
    ../../../bundles/default.nix
  ];

  # Set your time zone.
  time.timeZone = "America/New_York";

  # Select internationalization properties.
  i18n.defaultLocale = "en_US.UTF-8";
  i18n.extraLocaleSettings = {
    LC_ADDRESS = "en_US.UTF-8";
    LC_IDENTIFICATION = "en_US.UTF-8";
    LC_MEASUREMENT = "en_US.UTF-8";
    LC_MONETARY = "en_US.UTF-8";
    LC_NAME = "en_US.UTF-8";
    LC_NUMERIC = "en_US.UTF-8";
    LC_PAPER = "en_US.UTF-8";
    LC_TELEPHONE = "en_US.UTF-8";
    LC_TIME = "en_US.UTF-8";
  };

  environment.systemPackages = import ./pkgs.nix { inherit pkgs; };

  sops = lib.mkIf (config.host.default-sops != null) {
    defaultSopsFile = config.host.default-sops;
    age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];
  };

  programs.tmux = {
    enable = true;
  };

  # upstream hardcodes the greeting green around greetingLine, so recolour here
  environment.etc.issue.text = ''

    \e{lightblue}${config.services.getty.greetingLine}\e{reset}
    ${config.services.getty.helpLine}

  '';

}
