{ pkgs }:
let
  bulwark = import ./bulwark-server.nix { inherit pkgs; };
in
pkgs.testers.runNixOSTest {
  name = "bulwark";

  defaults.virtualisation.graphics = false;

  nodes.server = bulwark.node { };

  testScript =
    { nodes, ... }:
    bulwark.python nodes.server
    + ''
      start_all()
      wait_for_server()
      run_specs()
    '';
}
