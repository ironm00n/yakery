{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  inherit (config) xdg;
  cfg = config.bundles.emacs;

  doomemacs = pkgs.fetchFromGitHub {
    owner = "doomemacs";
    repo = "doomemacs";
    rev = "09f104795de1faa2cfed95694c1a37d0def1ec1b"; # release(modules): 25.10.0-dev
    hash = "sha256-yrf5f5OyODhxOV9+7HFyo1j7cgoE1DERu2xzA74RX90=";
  };

  doomDir = "${xdg.configHome}/doom";
  doomLocalDir = "${xdg.cacheHome}/doom";
  emacsDir = "${xdg.configHome}/emacs";
in
{
  options.bundles.emacs = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable (doom) emacs.";
    };
  };

  config = mkIf cfg.enable {
    home.sessionVariables = {
      DOOMDIR = doomDir;
      DOOMLOCALDIR = doomLocalDir;
    };

    home.packages = [
      (pkgs.writeShellScriptBin "doom" ''
        set -euo pipefail
        export DOOMDIR="''${DOOMDIR:-${doomDir}}"
        export DOOMLOCALDIR="''${DOOMLOCALDIR:-${doomLocalDir}}"
        exec "${emacsDir}/bin/doom" "$@"
      '')
    ];

    home.file.${emacsDir}.source = doomemacs;
    home.file."${doomDir}/init.el".source = symlink ./init.el;
    home.file."${doomDir}/packages.el".source = symlink ./packages.el;
    home.file."${doomDir}/config.el".source = symlink ./config.el;
  };
}
