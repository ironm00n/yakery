{
  inputs,
  lib,
  host,
  pkgs,
  pkgs-stable,
}:
{
  inherit pkgs-stable inputs;
  inherit host;
  # TODO: how to get home-manager's version of config?
  my-utils = import ./my-utils.nix {
    inherit lib pkgs inputs;
    inherit host;
    inherit (inputs.home-manager.lib) hm;
  };
}
