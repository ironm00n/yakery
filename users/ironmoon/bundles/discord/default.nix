{
  config,
  lib,
  my-lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-lib) mkDisableOption;
  cfg = config.bundles.discord;
in
{
  options.bundles.discord = {
    enable = mkEnableOption "Discord";
    vesktop = mkDisableOption "Vesktop";
    vencord = mkEnableOption "inject Vencord into official clients";
    official = mkEnableOption "official clients";
  };

  config = mkIf cfg.enable {
    home.packages =
      let
        discord = pkgs.discord.override {
          withVencord = cfg.vencord;
        };
        discord-ptb = pkgs.discord-ptb;
        discord-canary = pkgs.discord-canary.override {
          withOpenASAR = cfg.vencord;
          withVencord = cfg.vencord;
        };

        # fork of vgskye/ventex
        katex-plugin = pkgs.fetchFromGitHub {
          owner = "ironm00n";
          repo = "ventex";
          rev = "8ed8470a1589b8e1af6b5493ed449120e8ebfd2b";
          hash = "sha256-iXNdm75K3d24kwMCd7Gkv7cmudqLP2HO8nW8KqcE9V8=";
        };

        vesktop = import ../../../../packages/vesktop-with-plugins/package.nix {
          inherit pkgs;
          vesktopArgs = {
            withTTS = true;
            withMiddleClickScroll = true;
          };
          userPlugins = [ katex-plugin ];
        };
      in
      lib.optionals cfg.vesktop [
        vesktop
      ]
      ++ lib.optionals cfg.official [
        discord
        discord-ptb
        discord-canary
      ];
  };
}
