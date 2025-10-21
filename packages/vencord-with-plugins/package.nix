{
  pkgs,
  userPlugins ? [ ],
}:
let
  inherit (pkgs.lib) toList escapeShellArg;
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
in
pkgs.vencord.overrideAttrs (old: {

  preBuild =
    (old.preBuild or "")
    + ''
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
