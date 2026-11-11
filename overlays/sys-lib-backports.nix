final: prev:
let
  backport =
    name:
    { commit, ... }@fix:
    let
      inputs = removeAttrs fix [ "commit" ];
    in
    prev.${name}.overrideAttrs (
      old:
      let
        has = field: builtins.all (p: builtins.elem p (old.${field} or [ ])) inputs.${field};
      in
      final.lib.warnIf (builtins.all has (builtins.attrNames inputs))
        "overlays/sys-lib-backports: nixpkgs has ${name}'s fix (NixOS/nixpkgs ${commit}), drop it"
        (builtins.mapAttrs (field: added: old.${field} or [ ] ++ added) inputs)
    );
in
builtins.mapAttrs backport {
  anyrun = {
    commit = "b680fbe4";
    buildInputs = [ final.aws-lc ];
  };
  dufs = {
    commit = "09a6be7b";
    checkInputs = [ final.aws-lc ];
    nativeCheckInputs = [ final.pkg-config ];
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
