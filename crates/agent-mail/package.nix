{
  lib,
  rustPlatform,
  correspondents ? [ ],
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
  env.AGENT_MAIL_CORRESPONDENTS = lib.concatStringsSep "," correspondents;
  meta = {
    description = "Agent mail through Stalwart's JMAP, signed in with ZITADEL";
    mainProgram = "agent-mail";
  };
}
