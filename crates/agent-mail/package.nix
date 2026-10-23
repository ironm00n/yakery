{
  lib,
  rustPlatform,
  cacert,
  deployment,
  correspondents,
  correspondentsFor,
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
  # the tests build real HTTP clients, which load the system's roots though they never connect
  nativeCheckInputs = [ cacert ];
  env = import ./deployment-env.nix {
    inherit
      lib
      deployment
      correspondents
      correspondentsFor
      ;
  };
  meta = {
    description = "Agent mail through Stalwart's JMAP, signed in with ZITADEL";
    mainProgram = "agent-mail";
  };
}
