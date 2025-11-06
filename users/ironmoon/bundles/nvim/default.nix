{
  config,
  lib,
  my-utils,
  pkgs,
  pkgs-stable,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  inherit (config.xdg) configHome;
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
