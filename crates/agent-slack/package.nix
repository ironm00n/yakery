{ callPackage, cacert }:
callPackage ../rust.nix {
  crate = "agent-slack";
  # the tests build real HTTP clients, which load the system's roots though they never leave loopback
  nativeCheckInputs = [ cacert ];
}
