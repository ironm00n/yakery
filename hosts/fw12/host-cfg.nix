{ pkgs }:
{
  id = "framework-12-13th-gen-intel";
  hostname = "fw12";
  laptop = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
  lightweight = true;
  cpu-cores = 8;
  git-signing-key = "4EEAF95C719141D7E40BF84C017C3CE2B1EE3B29";
  appboxes = true;
  out-of-store-symlinks = true;
}
