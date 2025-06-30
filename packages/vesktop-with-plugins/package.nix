{
  pkgs,
  userPlugins ? [ ],
  vesktopArgs ? { },
}:
let
  vencord = import ../vencord-with-plugins/package.nix { inherit pkgs userPlugins; };
in
(pkgs.vesktop.override (
  vesktopArgs
  // {
    withSystemVencord = true;
    vencord = vencord;
  }
))
