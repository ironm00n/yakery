# One workspace member as a package, built from only the sources it compiles, or with `crate` unset every member's
# tests. Evaluation fails unless every manifest of its closure keeps manifest.nix's rules and every member names its
# targets' paths.
{
  lib,
  rustPlatform,
  pkgsStatic,
  crate ? null,
  static ? false,
  ...
}@args:
let
  workspace = import ./manifest.nix { inherit lib; };
  dirs = workspace.checked (if crate == null then workspace.members else workspace.closure crate);
  package = (workspace.manifest workspace.dirOf.${crate}).package;
  bins = (workspace.manifest workspace.dirOf.${crate}).bin or [ ];
  flags =
    if crate == null then
      [ "--workspace" ]
    else
      [
        "-p"
        crate
      ];
in
(if static then pkgsStatic.rustPlatform else rustPlatform).buildRustPackage (
  removeAttrs args [
    "lib"
    "rustPlatform"
    "pkgsStatic"
    "crate"
    "static"
  ]
  // {
    pname = if crate == null then "crates" else crate;
    version = if crate == null then "0" else package.version;
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions (
        [
          ./Cargo.toml
          ./Cargo.lock
        ]
        ++ map (dir: ./. + "/${dir}/Cargo.toml") workspace.members
        ++ map (dir: ./. + "/${dir}") dirs
      );
    };
    cargoLock.lockFile = ./Cargo.lock;
    cargoBuildFlags = flags;
    cargoTestFlags = flags;
    meta = lib.optionalAttrs (crate != null) (
      {
        inherit (package) description;
      }
      // lib.optionalAttrs (lib.length bins == 1) { mainProgram = (lib.head bins).name; }
    );
  }
  // lib.optionalAttrs (crate == null) {
    dontCargoBuild = true;
    checkType = "debug";
    installPhase = "touch $out";
  }
)
