{ lib, rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "agentd";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;
  meta.description = "The agent VM's root daemon and its host control client";
}
