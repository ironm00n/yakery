{ pkgs }:
{
  id = "desktop-2070super"; # TODO: get better id
  hostname = "desktop";
  nvidia = true;
  appboxes = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
  cpu-cores = 16;
  out-of-store-symlinks = true;
  git-signing-key = "2BFCA49FA307CBF3A19BDEBDDA16F613AAF76B3E";
}
