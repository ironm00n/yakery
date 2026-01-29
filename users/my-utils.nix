{
  host,
  pkgs,
  lib,
  inputs,
  ...
}:
let
  inherit (inputs.home-manager.lib) hm;
  inherit (lib) removePrefix escapeShellArg;
  inherit (lib.strings) optionalString;
  inherit (pkgs) runCommandLocal;
in
rec {
  # FIXME: this is copied from home-manager modules/files.nix, can it be extracted?
  mkOutOfStoreSymlink =
    path:
    let
      name = hm.strings.storeFileName (baseNameOf path);
    in
    runCommandLocal name { } "ln -s ${escapeShellArg path} $out";

  symlink =
    file:
    if host.out-of-store-symlinks then
      let
        rootNixPath = builtins.getEnv "ROOT_NIX_PATH";
        rootDir = optionalString (rootNixPath == "") "/etc/nixos";
        path = rootDir + removePrefix (toString inputs.self) (toString file);
      in
      mkOutOfStoreSymlink (builtins.trace path path)
    else
      file;
}
