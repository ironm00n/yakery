{ pkgs }:
{
  id = "framework-13-7040-amd";
  hostname = "fw13";
  laptop = true;
  fingerprint = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
  out-of-store-symlinks = true;
  # home-manager-nixos = false;
  home-manager-nixos = true;
}
