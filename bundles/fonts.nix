# TODO: fontconfig
# TODO: customize using custom emoji fonts
args@{
  config,
  lib,
  my-lib,
  options,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-lib) mkDisableOption;
  cfg = config.bundles.fonts;
  # their private-use glyphs are internal, and outrank Nerd Fonts as fallback
  pua-squatters = with pkgs; [ newcomputermodern ];
  strip-pua = pkg: ''
    <match target="scan">
      <test name="file" compare="contains"><string>${pkg}/</string></test>
      <edit name="charset" mode="assign">
        <minus>
          <name>charset</name>
          <charset>
            <range><int>0xe000</int><int>0xf8ff</int></range>
            <range><int>0xf0000</int><int>0x10ffff</int></range>
          </charset>
        </minus>
      </edit>
    </match>
  '';
in
{
  options.bundles.fonts = {
    enable = mkEnableOption "font customizations";
    emoji = mkDisableOption "emoji fonts";
  };

  config = mkIf cfg.enable {
    bundles.fontconfig-scan-cache.enable = true;
    fonts = {
      enableDefaultPackages = false;
      packages =
        let
          twemoji-colr = import ../packages/twemoji-colr/package.nix args;
          twemoji-cbdt = import ../packages/twemoji-cbdt/package.nix args;
        in
        lib.optionals cfg.emoji [
          twemoji-colr
          twemoji-cbdt
        ]
        ++ pua-squatters
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

          noto-fonts-lgc-plus
          symbola

          lmodern
          source-sans
        ]);
      fontconfig.defaultFonts =
        lib.genAttrs [ "sansSerif" "serif" "monospace" ] (
          k: options.fonts.fontconfig.defaultFonts.${k}.default ++ [ "Symbols Nerd Font" ]
        )
        // {
          emoji = [ "Twemoji COLR" ];
        };
      fontconfig.localConf = ''
        <?xml version="1.0"?>
        <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
        <fontconfig>
        ${lib.concatMapStrings strip-pua pua-squatters}
        </fontconfig>
      '';
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
