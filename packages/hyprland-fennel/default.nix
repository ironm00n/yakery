{
  runCommand,
  lua5_5,
  hyprland,
}:
let
  stubs = "${hyprland}/share/hypr/stubs/hl.meta.lua";
in
# docsets/hyprland.lua: fennel-ls's view of the `hl` API, from the stubs.
# macros/hl-keys.fnl: the spec keys this build's runtime reads, asked of the build itself.
runCommand "hyprland-fennel" { } ''
  mkdir -p $out/docsets $out/macros
  ${lua5_5.pkgs.fennel}/bin/fennel ${./docset.fnl} ${stubs} > $out/docsets/hyprland.lua

  cp ${./probe.fnl} probe.fnl
  probe() {
    env HOME=$PWD XDG_RUNTIME_DIR=$PWD HL_STUBS=${stubs} HL_KEYS_OUT=$out/macros/hl-keys.fnl "$@" \
      ${hyprland}/bin/Hyprland --verify-config -c $PWD/probe.fnl
  }
  iterating=$( (probe HL_PROBE=classify || true) \
    | sed -n "s/.*hl\.\([a-z_]*\): unknown field '__probe_unknown'.*/\1/p")
  probe HL_ITERATING="$iterating"
  test -s $out/macros/hl-keys.fnl
''
