{
  lib,
  rustPlatform,
  policy,
}:
rustPlatform.buildRustPackage {
  pname = "outlook-sync";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference ../. (lib.fileset.maybeMissing ../target);
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "outlook-sync"
  ];
  cargoTestFlags = [
    "-p"
    "outlook-sync"
  ];
  env.OUTLOOK_SYNC_POLICY = builtins.toJSON policy;
  meta = {
    description = "One Microsoft 365 calendar mirrored into one Stalwart calendar";
    mainProgram = "outlook-sync";
  };
}
