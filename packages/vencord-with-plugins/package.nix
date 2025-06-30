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
    rev = "f6d92e5024d2b1aaa9731406bf0405420a446436";
    hash = "sha256-ty5597LCYuQRcXVFD9yOpEUqMs2G0IrJr0iHhq+6P84=";
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
