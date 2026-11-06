#!/usr/bin/env bash
# PreToolUse(Bash) guard: `jj git push` runs no git hook, so gradual-borrows' pre-push check never ran
# before its pushes and CI mailed Owen 11 failures (friction 110); and agents' commit subjects ran to
# a median of 126 characters on 11-05. A push from a Claude Code session passes only through
# jj-push.sh, which checks the commits it sends against the commit width rule and runs the repo's
# pre-push hook first; a dry run passes.

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd == *push* ]] || exit 0

instead="Run the same command with its \`jj\` replaced by \`bash ~/.claude/hooks/jj-push.sh\`: it checks the commits it would send against the commit width rule and runs the repo's pre-push hook as \`git push\` would, and pushes only if both pass."
deny() {
  jq -cn --arg r "Blocked: $1" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

# git takes any long option cut short while it stays unambiguous: --no-veri and longer are --no-verify
skips_hooks() {
  local a
  for a in "${w[@]:1}"; do [[ ${#a} -ge 9 && --no-verify == "$a"* ]] && return; done
  return 1
}

check_git() {
  [[ ${w[0]##*/} == git && " ${w[*]:1} " == *" push "* ]] && skips_hooks &&
    deny "a session's \`git push\` runs the commit width hook, and --no-verify skips it. Push without it; the hook acts only on commits the remote doesn't have yet."
}

check_jj() {
  [[ ${w[0]##*/} == jj ]] || return
  local a i words=()
  for ((i = 1; i < ${#w[@]}; i++)); do
    a=${w[i]}
    case $a in
      --dry-run | -h | --help) return ;;
      -R | --repository | --at-op | --at-operation | --color | --config | --config-file) ((i++)) ;;
      -*) ;;
      *) words+=("$a") ;;
    esac
  done
  [[ ${words[0]-} == git && ${words[1]-} == push ]] || return
  deny "a push from a Claude Code session goes through jj-push.sh, and \`jj git push\` runs neither the width check nor a pre-push hook. $instead"
}

# a pass per command name, since after xargs each pass skips words up to its own name
cwd=$(dir_of "$(field .cwd)" "")
each_command check_jj "$cmd" "$cwd" jj
each_command check_git "$cmd" "$cwd" git

exit 0
