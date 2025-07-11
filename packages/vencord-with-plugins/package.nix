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
    rev = "643122e323fd9b36b456b42ef13e159f5b10015e";
    hash = "sha256-ojy4cRT4Nef8HF+uwxwjbrE210Dkq5yjqmk4tygiNKE=";
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
})
