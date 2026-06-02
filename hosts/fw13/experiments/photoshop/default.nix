{ pkgs, ... }:
let
  wine-adobe = pkgs.callPackage ./wine-adobe.nix { };
  photoshop = pkgs.callPackage ./launcher.nix { inherit wine-adobe; };
in
{
  environment.systemPackages = [ photoshop ];
}
