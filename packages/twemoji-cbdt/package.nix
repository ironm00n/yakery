# based on the twitter-color-emoji package (https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/by-name/tw/twitter-color-emoji/package.nix)
# acts as a fallback for platforms that don't support COLR/CPAL
# uses pkgs-stable to avoid excessive rebuilds
{ pkgs, pkgs-stable, ... }:

let
  inherit (pkgs) noto-fonts-color-emoji;
  inherit (pkgs-stable) lib stdenv fetchFromGitHub;

  version = "17.0.2-1";
  # Fork: removes (incorrect?) changes to family emojis (3a4779dbd4b12106a8594d4ce305dce1957c13cd)
  twemoji = fetchFromGitHub {
    name = "twemoji";
    owner = "ironm00n";
    repo = "twemoji";
    rev = "4c6904ff90f808f104f6086caf46be05c1ce1fae";
    hash = "sha256-Eeyzw5Ke6QJduC9il/vBMXiCtqTNK5mOvc9TPzica+8=";
  };
in
stdenv.mkDerivation {
  pname = "twemoji-cbdt";
  inherit version;

  srcs = [
    noto-fonts-color-emoji.src
    twemoji
  ];

  sourceRoot = noto-fonts-color-emoji.src.name;

  postUnpack = ''
    chmod -R +w ${twemoji.name}
    mv ${twemoji.name} ${noto-fonts-color-emoji.src.name}
  '';

  nativeBuildInputs = with pkgs-stable; [
    cairo
    imagemagick
    pkg-config
    pngquant
    (python3.withPackages (
      python-packages: with python-packages; [
        fonttools
        nototools
      ]
    ))
    which
    zopfli
  ];

  postPatch =
    let
      templateSubstitutions = lib.concatStringsSep "; " [
        # "s#Noto Color Emoji#Twemoji Color CBDT#"
        "s#Noto Color Emoji#Twitter Color Emoji#"
        # "s#NotoColorEmoji#TwemojiColorCBDT#"
        "s#NotoColorEmoji#TwitterColorEmoji#"
        ''s#Copyright .* Google Inc\.#Twitter, Inc and other contributors.#''
        "s# Version .*# ${version}#"
        "s#.*is a trademark.*##"
        ''s#Google, Inc\.#Twitter, Inc and other contributors#''
        "s#http://www.google.com/get/noto/#https://twemoji.twitter.com/#"
        "s#.*is licensed under.*#      Creative Commons Attribution 4.0 International#"
        "s#http://scripts.sil.org/OFL#http://creativecommons.org/licenses/by/4.0/#"
      ];
    in
    ''
      ${noto-fonts-color-emoji.postPatch}

      # sed '${templateSubstitutions}' NotoColorEmoji.tmpl.ttx.tmpl > TwemojiCBDT.tmpl.ttx.tmpl
      sed '${templateSubstitutions}' NotoColorEmoji.tmpl.ttx.tmpl > TwitterColorEmoji.tmpl.ttx.tmpl
      pushd ${twemoji.name}/assets/72x72/
      for png in *.png; do
          mv $png emoji_u''${png//-/_}
      done
      popd
    '';

  makeFlags = [
    # "EMOJI=TwemojiColorCBDT"
    "EMOJI=TwitterColorEmoji"
    "EMOJI_SRC_DIR=${twemoji.name}/assets/72x72"
    "BODY_DIMENSIONS=76x72"
    "BYPASS_SEQUENCE_CHECK=True"
  ];

  enableParallelBuilding = true;

  installPhase = ''
    # install -Dm644 TwemojiCBDT.ttf $out/share/fonts/truetype/TwemojiCBDT.ttf
    install -Dm644 TwitterColorEmoji.ttf $out/share/fonts/truetype/TwitterColorEmoji.ttf
  '';
}
