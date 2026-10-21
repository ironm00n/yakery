{
  lib,
  rustPlatform,
  deployment,
  correspondents,
}:
rustPlatform.buildRustPackage {
  pname = "agent-mail";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "agent-mail"
  ];
  cargoTestFlags = [
    "-p"
    "agent-mail"
  ];
  env = import ./deployment-env.nix { inherit lib deployment correspondents; };
  meta = {
    description = "Agent mail through Stalwart's JMAP, signed in with ZITADEL";
    mainProgram = "agent-mail";
  };
}
