#!/usr/bin/env bash
# PreToolUse(Bash) guard: denies flake commands whose flake resolves to a `path:` source. Outside
# a git work tree, `.` (or any bare path) is a path flake, and evaluating it copies the whole
# directory into the store, build outputs included; jj's secondary workspaces have no `.git`, so
# `nix develop -c cargo test` there made 36 copies of up to 4.1G each on 10-02 (84G). An explicit
# `path:`/`file:` ref copies the whole tree even inside a repo. `git+file`, registry and remote
# refs pass, as do `--expr` and `--file`, which name no flake, and a store path, which Nix reads as
# a store object before trying it as a flake. `nix` is checked wherever it is a word of a command, so
# wrappers such as `np-heavy.sh nix develop` count, and inside the quoted script of `sh -c`, but not
# in any other quoted argument (friction 103); a script that calls nix itself is not seen.

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd =~ nix|direnv ]] || exit 0

deny() {
  local advice="Run the flake from a git checkout instead (e.g. \`nix develop <main checkout> -c sh -c"
  advice+=" 'cd <workspace> && ...'\` or \`git+file://<repo>?rev=...\`), or call the tool directly"
  advice+=" (Lean: ~/.elan/bin/lake)."
  jq -cn --arg r "Blocked: $1 $advice" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

FLAKE_SUBCOMMANDS='develop|build|shell|run|print-dev-env|eval|flake|bundle|search|profile'
ONE_ARG_FLAGS=' --profile -I --include --max-jobs -j --cores --log-format --store --eval-store
  --system --reference-lock-file --output-lock-file --inputs-from -o --out-link --keep -k --unset
  -u --phase --update-input --priority --apply '
TWO_ARG_FLAGS=' --option --override-input --arg --argstr --redirect --override-flake '
STORE_PATH='^/nix/store/[0-9a-df-np-sv-z]{32}-[A-Za-z0-9+_?=-][A-Za-z0-9+._?=-]*'

check_dir() {
  [[ -n $1 ]] || deny "$2 runs in a directory the hook can't read off the line (a variable, substitution or \`cd -\`)."
  [[ -d $1 ]] || return 0
  git -C "$1" rev-parse --is-inside-work-tree >/dev/null 2>&1 ||
    deny "$2 evaluates \`$1\` as a path: flake (no .git there), which copies the whole directory into the store."
}

# As InstallableDerivedPath::parse: a path under the store, after the word's own symlinks but no
# others, names the store path it is in; with an `^outputs` spec it must be that store path.
store_object() {
  local p=$1 t
  [[ $2 == *^* ]] && { [[ $p =~ $STORE_PATH$ ]]; return; }
  while [[ $p != /nix/store/?* && -L $p ]]; do
    t=$(readlink -- "$p")
    [[ $t == /* ]] || t=${p%/*}/$t
    p=$(realpath -ms -- "$t")
  done
  [[ $p =~ $STORE_PATH(/|$) ]]
}

# A masked ref is a quoted word the hook can't read, most often an unlisted flag's value.
check_ref() {
  local ref=${1%%[#?^]*}
  case $ref in
    *"$mask"* | *:*) return ;;
    '' | .* | /* | '~'* | */*) ;;
    *) return ;;
  esac
  store_object "$(dir_of "${1%^*}" "$here" -s)" "$1" && return
  check_dir "$(dir_of "${ref:-.}" "$here")" "\`nix\` with flake \`${1:-.}\`"
}

check_nix() {
  local sub=$1 a skip=0 seen=0
  shift
  for a; do
    [[ $a == @(path|file):* ]] && deny "\`nix $sub\` is given \`$a\`, a path:/file: ref, which copies the whole tree into the store."
  done
  for a; do
    ((skip > 0)) && { skip=$((skip - 1)); continue; }
    case $a in
      -c | --command | --) break ;;
      --expr | --file | -f) return ;;
      -*)
        [[ $ONE_ARG_FLAGS == *" $a "* ]] && skip=1
        [[ $TWO_ARG_FLAGS == *" $a "* ]] && skip=2
        continue ;;
    esac
    [[ $sub == @(flake|profile) && $seen == 0 ]] && { seen=1; continue; }
    check_ref "$a"
    return
  done
  check_ref ''
}

check_command() {
  local i
  for ((i = 0; i < ${#w[@]}; i++)); do
    case ${w[i]##*/} in
      nix) [[ ${w[i + 1]-} == @($FLAKE_SUBCOMMANDS) ]] && check_nix "${w[@]:i+1}" ;;
      direnv) [[ ${w[i + 1]-} == exec ]] && check_dir "$(dir_of "${w[i + 2]-}" "$here")" "\`direnv exec\`" ;;
    esac
  done
}

cwd=$(field .cwd)
each_command check_command "$cmd" "$(dir_of "${cwd:-$PWD}" "")" 'nix|direnv'

exit 0
