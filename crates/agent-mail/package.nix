{
  callPackage,
  lib,
  cacert,
  deployment,
  correspondents,
  correspondentsFor,
}:
callPackage ../rust.nix {
  crate = "agent-mail";
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
}
