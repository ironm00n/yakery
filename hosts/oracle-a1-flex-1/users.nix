{ pkgs, config, ... }:
{
  programs.zsh.enable = true;
  users.users.ironmoon = {
    isNormalUser = true;
    description = "ironmoon";
    extraGroups = [
      "networkmanager"
      "wheel"
      "libvirtd"
      "docker"
      "kvm"
      "dialout"
      "input"
      "wireshark"
    ];
    shell = pkgs.zsh;
    openssh.authorizedKeys.keys = config.users.users.root.openssh.authorizedKeys.keys;
  };
}
