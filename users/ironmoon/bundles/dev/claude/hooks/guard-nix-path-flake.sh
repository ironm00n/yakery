#!/usr/bin/env bash
# PreToolUse(Bash) guard: denies flake commands whose flake resolves to a `path:` source. Outside
# a git work tree, `.` (or any bare path) is a path flake, and evaluating it copies the whole
# directory into the store, build outputs included; jj's secondary workspaces have no `.git`, so
# `nix develop -c cargo test` there made 36 copies of up to 4.1G each on 10-02 (84G). An explicit
# `path:`/`file:` ref copies the whole tree even inside a repo. `git+file`, registry and remote
# refs pass. Only command-line invocations are seen: a script that calls nix itself is not.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

deny() {
  local advice="Run the flake from a git checkout instead (e.g. \`nix develop <main checkout> -c sh -c"
  advice+=" 'cd <workspace> && ...'\` or \`git+file://<repo>?rev=...\`), or call the tool directly"
  advice+=" (Lean: ~/.elan/bin/lake)."
  jq -cn --arg r "$1 $advice" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
cwd=$(jq -r '.cwd // ""' <<<"$payload")
[[ -n "$cwd" ]] || cwd=$PWD
grep -Eq '\b(nix|direnv)\b' <<<"$cmd" || exit 0

# Quotes are dropped so `sh -c 'cd x && nix develop'` reads like the bare command it runs.
flat=$(tr -d "\"'" <<<"$cmd")

FLAKE_SUBCOMMANDS='develop|build|shell|run|print-dev-env|eval|flake|bundle|search|profile'
ONE_ARG_FLAGS=' --profile -I --include --max-jobs -j --cores --log-format --store --eval-store
  --system --reference-lock-file --output-lock-file --inputs-from -o --out-link --keep -k --unset
  -u --phase --update-input --expr --file -f --priority '
TWO_ARG_FLAGS=' --option --override-input --arg --argstr --redirect --override-flake '

# The directory a command at byte offset $1 of $flat runs in: the last `cd <dir>` before it.
dir_at() {
  local d=$cwd target
  target=$(head -c "$1" <<<"$flat" | grep -Eo '(^|[;&|(]|[[:space:]])cd[[:space:]]+[^;&|[:space:]]+' |
    tail -n1 | sed -E 's/.*cd[[:space:]]+//')
  [[ -n "$target" ]] || { printf '%s' "$d"; return; }
  target=${target/#\~/$HOME}
  [[ "$target" == /* ]] || target="$d/$target"
  printf '%s' "$target"
}

check_dir() {
  local dir=$1 what=$2
  [[ "$dir" == *'$'* ]] && deny "Blocked: $what runs in \`$dir\`, which can't be resolved before it runs."
  [[ -d "$dir" ]] || return 0
  git -C "$dir" rev-parse --is-inside-work-tree >/dev/null 2>&1 ||
    deny "Blocked: $what evaluates \`$(realpath -m "$dir")\` as a path: flake (no .git there), which copies the whole directory into the store."
}

check_ref() {
  local ref=${2%%#*} base=$1
  ref=${ref%%\?*}
  case $ref in
    path:* | file:*) deny "Blocked: \`$2\` is a path:/file: flake ref, which copies the whole tree into the store." ;;
    *:*) return 0 ;;
    '' | .* | /* | \~* | */*) ;;
    *) return 0 ;;
  esac
  ref=${ref:-.}
  ref=${ref/#\~/$HOME}
  [[ "$ref" == /* ]] || ref="$base/$ref"
  check_dir "$ref" "\`nix\` with flake \`${2:-.}\`"
}

while IFS=: read -r off sub; do
  rest=$(tail -c +"$((off + 1))" <<<"$flat" | sed -E 's/^nix[[:space:]]+[^[:space:]]+//; s/(;|&&|\|\||\|).*//')
  grep -Eq '(^|[[:space:]])(path|file):' <<<"$rest" &&
    deny "Blocked: \`nix $sub\` is given a path:/file: ref, which copies the whole tree into the store."
  base=$(dir_at "$off")
  read -ra words <<<"$rest"
  ref='' skip=0 seen_sub=0
  for w in "${words[@]}"; do
    ((skip > 0)) && { skip=$((skip - 1)); continue; }
    case $w in
      -c | --command | --) break ;;
      -*)
        [[ "$ONE_ARG_FLAGS" == *" $w "* ]] && skip=1
        [[ "$TWO_ARG_FLAGS" == *" $w "* ]] && skip=2
        continue ;;
    esac
    if [[ $sub =~ ^(flake|profile)$ && $seen_sub == 0 ]]; then seen_sub=1; continue; fi
    ref=$w
    break
  done
  check_ref "$base" "$ref"
done < <(grep -Ebo "\bnix[[:space:]]+($FLAKE_SUBCOMMANDS)\b" <<<"$flat" | sed -E 's/nix[[:space:]]+//')

while IFS=: read -r off dir; do
  dir=${dir/#\~/$HOME}
  [[ "$dir" == /* ]] || dir="$(dir_at "$off")/$dir"
  check_dir "$dir" "\`direnv exec\`"
done < <(grep -Ebo '\bdirenv[[:space:]]+exec[[:space:]]+[^[:space:];&|]+' <<<"$flat" |
  sed -E 's/direnv[[:space:]]+exec[[:space:]]+//')

exit 0
