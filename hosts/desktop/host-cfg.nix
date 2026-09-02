{ pkgs }:
{
  id = "desktop-2070super"; # TODO: get better id
  hostname = "desktop";
  nvidia = true;
  appboxes = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
  cpu-cores = 16;
  out-of-store-symlinks = true;
}
