{ pkgs }:
{
  id = "desktop-2070super"; # TODO: get better id
  hostname = "desktop";
  nvidia = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
}
