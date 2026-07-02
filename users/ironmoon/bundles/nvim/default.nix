{
  config,
  lib,
  pkgs,
  inputs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.nvim;
in
{
  options.bundles.nvim = {
    enable = mkEnableOption "neovim";
  };

  config = mkIf cfg.enable {
    programs.nixvim = (import ./config.nix { inherit pkgs; }) // {
      enable = true;
      nixpkgs.source = inputs.nixpkgs;
    };
  };
}
