{ lib }:
{
  sops = import ./sops.nix { inherit lib; };
  systemd = import ./systemd.nix { inherit lib; };
  netbird = import ./netbird.nix;
  borg = import ./borg.nix { inherit lib; };
  stalwart = import ./stalwart.nix { inherit lib; };

  mkDisableOption =
    name:
    lib.mkEnableOption name
    // {
      default = true;
      example = false;
    };
}
