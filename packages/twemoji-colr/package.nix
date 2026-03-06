# upstream:
# - https://github.com/jdecked/twemoji
# - https://github.com/mozilla/twemoji-colr
# uses pkgs-stable to avoid excessive rebuilds
{ inputs, system, ... }:
let
  pkgs = inputs.nixpkgs-24_11.legacyPackages.${system};
  inherit (pkgs) fetchFromGitHub buildNpmPackage;

  version = "17.0.2-1";
  twemoji = fetchFromGitHub {
    name = "twemoji";
    owner = "ironm00n";
    repo = "twemoji";
    rev = "4c6904ff90f808f104f6086caf46be05c1ce1fae";
    hash = "sha256-Eeyzw5Ke6QJduC9il/vBMXiCtqTNK5mOvc9TPzica+8=";
  };

  # changes in fork:
  # - change FONT_NAME to "Twemoji COLR" in Makefile
  # - change font name Gruntfile.js
  # - update package-lock.json to have `integrity` and `resolved` fields
  # - fixes terminal rendering in kitty
  twemoji-colr = fetchFromGitHub {
    name = "twemoji-colr";
    owner = "ironm00n";
    repo = "twemoji-colr";
    rev = "a32fd02b30df24d9abb18270d3b91d08f5a74340";
    hash = "sha256-XDu6Z9iW+/I6xT02hMU7Btb3Ovu0jUrilUODv9oAuws=";
  };
in
buildNpmPackage (final: {
  pname = "twemoji-colr";
  inherit version;

  srcs = [
    twemoji
    twemoji-colr
  ];

  sourceRoot = twemoji-colr.name;

  npmDepsHash = "sha256-fZ5Xd70r0t6WMjkAYCktasAuvif0KIIMz6L1Swvznpc=";

  nativeBuildInputs = with pkgs; [
    nodejs
    node-gyp
    pkg-config
    fontforge
    python3Packages.fonttools
    python3Packages.distutils
    zip
    unzip
    which
    perl
  ];

  buildInputs = with pkgs; [
    pixman
    cairo
    pango
  ];

  buildPhase = ''
    runHook preBuild

    zip -r twe-svg.zip ../twemoji/assets/svg
    make
    mv "build/Twemoji COLR.ttf" build/twemoji-colr.ttf

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    install -Dm644 build/twemoji-colr.ttf $out/share/fonts/truetype/twemoji-colr.ttf

    runHook postInstall
  '';
})
