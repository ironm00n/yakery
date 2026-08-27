{
  config,
  lib,
  pkgs,
  inputs,
  my-utils,
  ...
}:
let
  inherit (lib)
    getExe
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  inherit (my-utils) aliasExe;
  cfg = config.bundles.nvim;
in
{
  options.bundles.nvim = {
    enable = mkEnableOption "neovim";

    package = mkOption {
      type = types.package;
      readOnly = true;
      description = "The configured nixvim, exposed so it can be aliased under other names.";
      default = import ../../../../nix/nvim/default.nix {
        inherit pkgs;
        inherit (inputs.nixvim.legacyPackages.${pkgs.stdenv.hostPlatform.system}) makeNixvimWithModule;
      };
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ (aliasExe "nixvim" cfg.package) ];

    home.sessionVariables = {
      EDITOR = getExe cfg.package;
      VISUAL = getExe cfg.package;
    };
  };
}
