{
  pkgs,
  userPlugins ? [ ],
}:
let
  inherit (pkgs.lib) toList escapeShellArg;
  resolvedPlugins = map (
    plugin:
    let
      path = if plugin ? outPath then plugin.outPath else plugin;
      name = baseNameOf (toString path);
    in
    {
      inherit path name;
    }
  ) (toList userPlugins);
in
pkgs.vencord.overrideAttrs (old: {

  src = pkgs.fetchFromGitHub {
    owner = "Vendicated";
    repo = "Vencord";
    rev = "a33e81d1cbd7ab50c0b1e1446c925bf259e671fc";
    hash = "sha256-7JT8BMKUhIwYMkIwr2mD8IQLDpldcDtAKh6R1tbAKMw=";
    # hash = pkgs.lib.fakeHash;
  };

  preBuild =
    (old.preBuild or "")
    + ''
      mkdir -p src/userplugins
      ${pkgs.lib.concatStringsSep "\n" (
        map (
          { path, name }:
          ''
            mkdir -p src/userplugins/${name}
            cp -r ${escapeShellArg path}/* src/userplugins/${name}/
          ''
        ) resolvedPlugins
      )}
    '';

  patches = (old.patches or [ ]) ++ [
    ./remove-support-warning.patch
  ];

  patchFlags = (old.patches or [ ]) ++ [
    "-p1"
    "--no-backup-if-mismatch"
  ];
})
