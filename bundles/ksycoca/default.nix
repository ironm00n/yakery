{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
  cfg = config.bundles.ksycoca;
in
{
  options.bundles.ksycoca = {
    enable = mkEnableOption "a KSycoca that notices a new system generation";
    package = mkOption {
      type = types.package;
      default = pkgs.kdePackages.kservice;
      description = "The kservice the closure links; swapped for the patched build without rebuilding its dependents.";
    };
  };

  config = mkIf cfg.enable {
    system.replaceDependencies.replacements = [
      {
        oldDependency = cfg.package;
        newDependency = cfg.package.overrideAttrs (old: {
          patches = old.patches ++ [ ./symlink-chain-mtime.patch ];
        });
      }
    ];
  };
}
