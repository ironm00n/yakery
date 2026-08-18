# Build of the `slurm-ci` dispatcher.
#
# Deployment glue is low-churn, so this is plain buildRustPackage rather than
# crane: no extra flake inputs, no toolchain overlay. `pkgsStatic.callPackage`-ing
# this yields a fully static (musl) binary that copies cleanly onto the
# (non-Nix) Explorer login node, and runs fine on the micro too.
{
  lib,
  rustPlatform,
  busybox,
}:
rustPlatform.buildRustPackage {
  pname = "slurm-ci";
  version = (lib.importTOML ./Cargo.toml).package.version;

  # `../.` is the workspace root (crates/): the workspace manifest + lock must be
  # in scope even though we build only the `slurm-ci` member.
  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "slurm-ci"
  ];
  cargoTestFlags = [
    "-p"
    "slurm-ci"
  ];
  # tests/execution_stack.rs fakes the bootstrap store's tools with busybox
  # scripts; under pkgsStatic this is the static busybox it needs.
  env.SLURM_CI_TEST_BUSYBOX = "${busybox}/bin/busybox";

  meta = {
    description = "Forgejo Actions → Explorer Slurm CI dispatcher";
    mainProgram = "slurm-ci";
  };
}
