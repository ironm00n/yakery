{
  inputs,
  lib,
  my-lib,
  host,
  mv,
  pkgs,
  pkgs-stable,
  pkgs-master,
}:
{
  inherit inputs my-lib host;
  inherit mv;
  inherit pkgs pkgs-stable pkgs-master;
  # TODO: how to get home-manager's version of config?
  my-utils = import ./my-utils.nix {
    inherit lib pkgs inputs;
    inherit host;
    inherit (inputs.home-manager.lib) hm;
  };
}
