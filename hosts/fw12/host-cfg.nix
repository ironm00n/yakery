{ pkgs }:
{
  id = "framework-12-13th-gen-intel";
  hostname = "fw12";
  laptop = true;
  additional-user-pkgs = import ./additional-user-pkgs.nix { inherit pkgs; };
  lightweight = true;
}
