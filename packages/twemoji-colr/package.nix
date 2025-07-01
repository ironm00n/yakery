# upstream:
# - https://github.com/jdecked/twemoji
# - https://github.com/mozilla/twemoji-colr
# uses pkgs-stable to avoid excessive rebuilds
{ inputs, system, ... }:
let
  pkgs = inputs.nixpkgs-24_11.legacyPackages.${system};

  twemoji = pkgs.fetchFromGitHub {
    name = "twemoji";
    owner = "IRONM00N";
    repo = "twemoji";
    rev = "188c19556b1c41c4009dffdb36ad19dbdbf50eb2";
    hash = "sha256-wZLuob2Ap9Dm9c0T8rl/HgldrPJz+CXORu9UkiME3e8=";
  };

  # changes in fork:
  # - change FONT_NAME to "Twemoji COLR" in Makefile
  # - change font name Gruntfile.js
  # - update package-lock.json to have `integrity` and `resolved` fields
  twemoji-colr = pkgs.fetchFromGitHub {
    name = "twemoji-colr";
    owner = "IRONM00N";
    repo = "twemoji-colr";
    rev = "b2f2b905de0d484336e4de8859f449afa111f089";
    hash = "sha256-iRHtmyCEzGYS1US4uB2laTmC6OhYO0FL0tJ/O1xhxcs=";
  };
in
pkgs.buildNpmPackage (final: {
  pname = "twemoji-colr";
  version = "15.1.1";

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
