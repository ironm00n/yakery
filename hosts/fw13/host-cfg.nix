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
  cpu-cores = 16;
  appboxes = true;
  git-signing-key = "05E4B5CCAC4D2E405C14CC24047E6905B5BB8193";
}
