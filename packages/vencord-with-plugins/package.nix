{
  pkgs,
  userPlugins ? [ ],
}:
let
  inherit (pkgs.lib)
    toList
    escapeShellArg
    versionOlder
    warnIfNot
    ;
  resolvedPlugins =
    (toList userPlugins)
    |> map (
      plugin:
      let
        path = if plugin ? outPath then plugin.outPath else plugin;
        name = baseNameOf (toString path);
      in
      {
        inherit path name;
      }
    );
  vencord = pkgs.vencord.overrideAttrs (
    old:
    let
      version = "1.15.16";
    in
    warnIfNot (versionOlder old.version version)
      "vencord-with-plugins: nixpkgs has vencord ${old.version}, drop the ${version} bump"
      {
        inherit version;
        src = old.src.override { hash = "sha256-aUJubLJpyLAZWiOezqFxPY6oVPgWVPQpajFqrJvrO9U="; };
        # nixpkgs' fix-deps.patch and postPatch pin @types/react, which 1.15.16 moved into a pnpm catalog
        patches = [ ];
        postPatch = "";
        pnpmDeps = old.pnpmDeps.overrideAttrs {
          outputHash = "sha256-HS8uZNepvDefjrsG3buVLFCVv7sHvazcwdpcT3EQ/78=";
        };
      }
  );
in
vencord.overrideAttrs (old: {

  preBuild = (old.preBuild or "") + ''
    mkdir -p src/userplugins
    ${
      resolvedPlugins
      |> map (
        { path, name }:
        ''
          mkdir -p src/userplugins/${name}
          cp -r ${escapeShellArg path}/* src/userplugins/${name}/
        ''
      )
      |> pkgs.lib.concatStringsSep "\n"
    }
  '';

  patches = (old.patches or [ ]) ++ [
    ./remove-support-warning.patch
  ];

  patchFlags = (old.patchFlags or [ ]) ++ [
    "-p1"
    "--no-backup-if-mismatch"
  ];
})
