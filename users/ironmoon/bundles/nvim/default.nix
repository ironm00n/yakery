{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  cfg = config.bundles.nvim;
in
{
  options.bundles.nvim = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable neovim.";
    };
  };

  config = mkIf cfg.enable {
    programs.nixvim = (import ./config.nix { inherit pkgs; }) // {
      enable = true;
    };
  };
}
