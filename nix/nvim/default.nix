{ makeNixvimWithModule, pkgs }:

makeNixvimWithModule {
  inherit pkgs;
  module = import ../../users/ironmoon/bundles/nvim/config.nix;
}
