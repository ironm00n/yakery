# kitty's built-in 16-color palette, read out of the package so it can't drift.
{ pkgs }:
let
  dump = pkgs.runCommand "kitty-default-palette.json" { nativeBuildInputs = [ pkgs.kitty ]; } ''
    kitty +runpy '
    import json
    from kitty.options.types import defaults
    palette = []
    for i in range(16):
        palette.append(getattr(defaults, "color%d" % i).as_sharp[1:])
    print(json.dumps(palette))
    ' > $out
  '';
in
builtins.fromJSON (builtins.readFile dump)
