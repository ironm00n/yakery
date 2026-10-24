# Static musl, so one binary serves every role, the cluster's dispatch and build on its non-Nix hosts included.
{ callPackage, pkgsStatic }:
callPackage ../rust.nix {
  crate = "slurm-ci";
  static = true;
  # tests/execution_stack.rs fakes the bootstrap store's tools with busybox scripts, which must be static too
  env.SLURM_CI_TEST_BUSYBOX = "${pkgsStatic.busybox}/bin/busybox";
}
