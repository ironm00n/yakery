{
  lib,
  fetchFromGitHub,
  stdenv,
  stalwart,
  nix-update-script,
  python314,
  python314Packages
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "spam-filter";
  version = "3.0.2";

  src = fetchFromGitHub {
    owner = "stalwartlabs";
    repo = "spam-filter";
    tag = "v${finalAttrs.version}";
    hash = "sha256-dMHfVzSTP/J+ohBIOIXZ3eKPVK+gnLZ0ifq5/u2EBFE=";
  };

  buildInputs = [
    python314
    python314Packages.tomli
  ];

  buildPhase = ''
    python generate_rules_json.py
  '';

  installPhase = ''
    mkdir -p $out
    cp spam-filter.toml $out/
    cp spam-filter-rules.json $out/
  '';

  passthru = {
    updateScript = nix-update-script { };
  };

  meta = {
    description = "Spam filter module for the Stalwart server";
    homepage = "https://github.com/stalwartlabs/spam-filter";
    changelog = "https://github.com/stalwartlabs/spam-filter/blob/${finalAttrs.src.tag}/CHANGELOG.md";
    license = with lib.licenses; [
      mit
      asl20
    ];
    inherit (stalwart.meta) maintainers;
  };
})

