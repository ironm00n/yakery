# TODO: fontconfig
# TODO: customize using custom emoji fonts
args@{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    types
    mkOption
    mkIf
    ;
  cfg = config.bundles.fonts;
in
{
  options.bundles.fonts = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable font customizations.";
    };
  };

  config = mkIf cfg.enable {
    fonts = {
      enableDefaultPackages = false;
      packages =
        let
          twemoji-colr = import ../packages/twemoji-colr/package.nix args;
          twemoji-cbdt = import ../packages/twemoji-cbdt/package.nix args;
        in
        [

          twemoji-colr
          twemoji-cbdt

        ]
        ++ (with pkgs; [
          # default minus noto-fonts-color-emoji
          dejavu_fonts
          freefont_ttf
          gyre-fonts
          liberation_ttf
          unifont

          # other
          fira-code
          fira-code-symbols
          nerd-fonts.fira-code
          nerd-fonts.jetbrains-mono
          nerd-fonts.symbols-only
          font-awesome
          source-code-pro
          lato
          open-sans
        ]);
      fontconfig.defaultFonts.emoji = [ "Twemoji COLR" ];
      # fontconfig.localConf = ''
      #   <?xml version="1.0"?>
      #   <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
      #   <fontconfig>
      #     <alias binding="same">
      #       <family>Twemoji Color CBDT</family>
      #       <default><family>emoji</family></default>
      #     </alias>
      #     <alias binding="same">
      #       <family>emoji</family>
      #       <prefer>
      #         <family>Twemoji Color COLR</family>
      #         <family>Twemoji Color CBDT</family>
      #       </prefer>
      #     </alias>
      #   </fontconfig>
      # '';
    };
  };
}
