{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.sec;
in
{
  options.bundles.sec = {
    enable = mkEnableOption "security stuff";
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
