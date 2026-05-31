{ runCommand, writeShellScript, comma }:
let
  picker = writeShellScript "noninteractive-picker" ''
    cands=$(cat)
    n=$(printf '%s\n' "$cands" | grep -c .)
    if [ "$n" -gt 1 ]; then
      {
        printf 'multiple packages provide this command:\n'
        printf '%s\n' "$cands"
        printf 'pick one and run instead: nix run nixpkgs#<pkg> -- <args>\n'
      } >&2
      exit 1
    fi
    printf '%s\n' "$cands"
  '';

  wrapper = writeShellScript "comma-noninteractive-wrapper" ''
    exec ${comma}/bin/comma -P ${picker} "$@"
  '';
in
runCommand "comma-noninteractive" { } ''
  mkdir -p "$out/bin"
  ln -s ${wrapper} "$out/bin/,,"
''
