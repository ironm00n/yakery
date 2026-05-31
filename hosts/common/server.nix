{
  config,
  lib,
  pkgs,
  ...
}:
{
  imports = [
    ./networked.nix
  ];

  nix.gc = {
    automatic = true;
    dates = "weekly";
    options = "--delete-older-than 30d";
  };

  users.users.root.openssh.authorizedKeys.keys = import ./admin-ssh-keys.nix;
}
