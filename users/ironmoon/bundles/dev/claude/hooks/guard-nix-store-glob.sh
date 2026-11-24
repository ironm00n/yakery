#!/usr/bin/env bash
# PreToolUse(Bash) guard: denies a glob over /nix/store's top level. To match one the shell lists every store path
# and holds the matches: `ls -d /nix/store/*/lib/rustlib/…` grew zsh to 28.5 GiB on 11-07. A word counts when the dir
# its first globbing part lists is /nix/store, after the line's `cd`s and symlinks. A quoted or escaped pattern, which
# a program reads itself, passes, as do a glob under one store path and the patterns of `[[ … ]]` and `case`, which
# never expand to paths.

set -uo pipefail
quoted_globs=1
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd == *[*?[]* ]] || exit 0

deny() {
  local r="Blocked: \`$1\` globs over /nix/store's top level, so the shell would list and hold every store path."
  r+=" Name the path instead: \`nix eval --raw <installable>.outPath\`, \`nix path-info\` or \`command -v <tool>\`,"
  r+=" or look it up with \`fd -d 1 <name> /nix/store\`."
  jq -cn --arg r "$r" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

# The dir word $1's first globbing part lists, from the dir the command runs in; nothing when the word holds no glob
# or the line doesn't say where it points.
listed() {
  local lead=${1%%[*?[]*}
  [[ $lead == "$1" ]] && return
  [[ $lead == */* ]] && lead=${lead%/*} || lead=.
  dir_of "${lead:-/}" "$here"
}

in_case=
check_command() {
  local word
  case ${w[0]} in
    'case') in_case=1 ;;
    'esac') in_case= ;;
    '[[') ;;
    *)
      [[ -z $in_case ]] || return 0
      for word in "${w[@]}"; do
        [[ $(listed "$word") == /nix/store ]] && deny "$word"
      done ;;
  esac
}

cwd=$(field .cwd)
each_command check_command "$cmd" "$(dir_of "${cwd:-$PWD}" "")" '*'

exit 0
