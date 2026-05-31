{ lib }:
{
  sops = import ./sops.nix { inherit lib; };
  systemd = import ./systemd.nix { inherit lib; };

  mkDisableOption =
    name:
    lib.mkEnableOption name
    // {
      default = true;
      example = false;
    };
}
