{ config, ... }:
{
  assertions = [
    {
      assertion =
        !(config.services.forgejo.enable && config.services.forgejo.package.vmTestOnly or false);
      message = "services.forgejo.package is nix/tests/forgejo-package.nix's build, which skips a test";
    }
  ];
}
