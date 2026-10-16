{ lib, pkgs }:
pkgs.writeText "agents.json" (
  builtins.toJSON (lib.mapAttrs (_: agent: agent.uid) (import ./agents.nix { inherit lib; }))
)
