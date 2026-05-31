{ inputs, lib, useSecrets }:
[
  inputs.nixvim.homeModules.nixvim
  # needed even when not using full kde (konsole, dolphin, etc)
  inputs.plasma-manager.homeModules.plasma-manager
]
++ lib.optionals useSecrets [
  inputs.sops-nix.homeManagerModules.sops
]
