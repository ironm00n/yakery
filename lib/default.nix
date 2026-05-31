{ lib }:
{
  sops = import ./sops.nix { inherit lib; };
  systemd = import ./systemd.nix { inherit lib; };
  netbird = import ./netbird.nix;

  mkDisableOption =
    name:
    lib.mkEnableOption name
    // {
      default = true;
      example = false;
    };
}
