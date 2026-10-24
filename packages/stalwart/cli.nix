{
  lib,
  rustPlatform,
  fetchFromGitHub,
  stalwart,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "stalwart-cli";
  version = "1.0.13";

  src = fetchFromGitHub {
    owner = "stalwartlabs";
    repo = "cli";
    rev = "v${finalAttrs.version}";
    hash = "sha256-8HiFxJT5mGZp/9al1y8Ycm5im+E0PMDhw2+XeJAabTY=";
  };

  cargoHash = "sha256-Lj7wBLMqN4xP2GXicFGgIxkPrGi7F8i9KBSfFSn+KNk=";

  doCheck = false;

  meta = {
    inherit (stalwart.meta) license homepage maintainers;
    description = "Stalwart mail & collaboration server CLI";
    mainProgram = "stalwart-cli";
    changelog = "https://github.com/stalwartlabs/cli/blob/${finalAttrs.version}/CHANGELOG.md";
  };
})

