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
  cfg = config.bundles.sec;
in
{
  options.bundles.sec = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable security stuff.";
    };
  };

  config = mkIf cfg.enable {
    home.packages = with pkgs; [
      postman
      ghidra
      burpsuite
      metasploit
      ida-free
    ];
  };
}
