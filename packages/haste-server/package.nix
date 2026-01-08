# adapted from: https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/by-name/ha/haste-server/package.nix

{
  buildNpmPackage,
  fetchFromGitHub,
}:

buildNpmPackage {
  pname = "haste-server";
  version = "2026-01-05-cc61004";

  src = fetchFromGitHub {
    owner = "TanzaniteBot";
    repo = "haste-server";
    rev = "cc61004d4c36b168626f41c1493a3b9a2d6dc7db";
    hash = "sha256-c9bJXrWNDSchkbwYDa6dYpCkMODjl36khl0HHeXc0oo=";
  };

  npmDepsHash = "sha256-j86kYMJMtCXQsQX4PnFtGLFud6iI7xmRjDi8qf38Yyc=";

  dontNpmBuild = true;

  postInstall = ''
    install -Dt "$out/share/haste-server" about.md
    rm -rf "$out/lib/node_modules/haste/node_modules/.bin/"
  '';
}
