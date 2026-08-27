{
  host,
  pkgs,
  lib,
  inputs,
  ...
}:
let
  inherit (inputs.home-manager.lib) hm;
  inherit (lib) getExe removePrefix escapeShellArg;
  inherit (pkgs) runCommandLocal;
in
rec {
  # nixpkgs has `linkFarm` as the primitive, but nothing that reads as
  # "same package, different command name" and carries mainProgram over
  aliasExe =
    name: pkg:
    runCommandLocal name { meta.mainProgram = name; } ''
      mkdir -p $out/bin
      ln -s ${escapeShellArg (getExe pkg)} $out/bin/${escapeShellArg name}
    '';

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
        rootNixPath = builtins.getEnv "ROOT_NIXOS_PATH";
        rootDir = if rootNixPath == "" then "/etc/nixos" else rootNixPath;
        path = rootDir + removePrefix (toString inputs.self) (toString file);
      in
      mkOutOfStoreSymlink (builtins.trace path path)
    else
      file;
}
