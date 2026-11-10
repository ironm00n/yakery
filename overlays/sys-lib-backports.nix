final: prev:
let
  backport =
    name:
    {
      commit,
      buildInputs ? [ ],
      nativeBuildInputs ? [ ],
    }:
    prev.${name}.overrideAttrs (
      old:
      let
        has = field: builtins.all (p: builtins.elem p (old.${field} or [ ]));
      in
      final.lib.warnIf (has "buildInputs" buildInputs && has "nativeBuildInputs" nativeBuildInputs)
        "overlays/sys-lib-backports: nixpkgs has ${name}'s fix (NixOS/nixpkgs ${commit}), drop it"
        {
          buildInputs = old.buildInputs or [ ] ++ buildInputs;
          nativeBuildInputs = old.nativeBuildInputs or [ ] ++ nativeBuildInputs;
        }
    );
in
builtins.mapAttrs backport {
  anyrun = {
    commit = "b680fbe4";
    buildInputs = [ final.aws-lc ];
  };
  tuicr = {
    commit = "c8056330";
    buildInputs = [
      final.libgit2
      final.oniguruma
    ];
    nativeBuildInputs = [ final.pkg-config ];
  };
}
