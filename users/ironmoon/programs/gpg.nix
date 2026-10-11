{ lib, host, ... }:
let
  keysIn =
    dir: trust:
    lib.mapAttrsToList (name: _: {
      source = dir + "/${name}";
      inherit trust;
    }) (lib.filterAttrs (_: type: type == "regular") (builtins.readDir dir));
in
{
  enable = true;
  settings = lib.optionalAttrs (host.git-signing-key != null) {
    default-key = host.git-signing-key;
  };
  publicKeys = keysIn ../resources/gpg "ultimate" ++ keysIn ../resources/gpg/retired null;
}
