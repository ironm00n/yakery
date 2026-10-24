{
  lib,
  rustPlatform,
  tzdata,
  policy,
  crashPoints ? false,
}:
rustPlatform.buildRustPackage {
  pname = "scheduler";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference ../. (lib.fileset.maybeMissing ../target);
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "scheduler"
  ];
  cargoTestFlags = [
    "-p"
    "scheduler"
  ];
  buildFeatures = lib.optional crashPoints "crash-points";
  env = {
    SCHEDULER_POLICY = builtins.toJSON policy;
    TZDIR = "${tzdata}/share/zoneinfo";
  };
  meta = {
    description = "A public page of busy times and one-time booking links on Stalwart";
    mainProgram = "scheduler";
  };
}
