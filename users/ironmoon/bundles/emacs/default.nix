{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  inherit (config) xdg;
  cfg = config.bundles.emacs;

  # TODO: pin to versioned releases on next release (> 2.1.0)
  doomemacs = pkgs.fetchFromGitHub {
    owner = "doomemacs";
    repo = "doomemacs";
    rev = "6be3337b49867bd86f90fe5ca4beeb6b38afaddb"; # fix(evil): tab ex commands
    hash = "sha256-7ErKUgw6Ch7hP1oBjMSos8xXRD+rxxjaOldRn+TcClo=";
  };

  doomDir = "${xdg.configHome}/doom";
  doomLocalDir = "${xdg.cacheHome}/doom";
  emacsDir = "${xdg.configHome}/emacs";
in
{
  options.bundles.emacs = {
    enable = mkEnableOption "(doom) emacs";
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
