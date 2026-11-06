#!/usr/bin/env bash
# PreToolUse(Bash) guard: `jj git push` runs no git hook, so gradual-borrows' pre-push check never ran
# before its pushes and CI mailed Owen 11 failures (friction 110). Where the repo has a pre-push hook
# (lib/jj.sh), a push passes only through jj-push.sh, which runs the hook first; a dry run passes.

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"
. "${BASH_SOURCE[0]%/*}/lib/jj.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd == *jj* && $cmd == *push* ]] || exit 0

instead="Run the same command with its \`jj\` replaced by \`bash ~/.claude/hooks/jj-push.sh\`: it runs the hook as \`git push\` would and pushes only if the hook passes."
deny() {
  jq -cn --arg r "Blocked: $1 $instead" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

check_push() {
  [[ ${w[0]##*/} == jj ]] || return
  local a i r=$here words=() root hook
  for ((i = 1; i < ${#w[@]}; i++)); do
    a=${w[i]}
    case $a in
      --dry-run | -h | --help) return ;;
      -R | --repository) ((i++)); r=$(dir_of "${w[i]-}" "$here") ;;
      --repository=*) r=$(dir_of "${a#*=}" "$here") ;;
      -R?*) r=$(dir_of "${a#-R}" "$here") ;;
      --at-op | --at-operation | --color | --config | --config-file) ((i++)) ;;
      -*) ;;
      *) words+=("$a") ;;
    esac
  done
  [[ ${words[0]-} == git && ${words[1]-} == push ]] || return
  [[ -n $r ]] || deny "the hook can't read off the line which repo this push runs in (a variable, substitution, quoted path with blanks or \`cd -\`), so it can't tell whether the repo has a pre-push hook."
  root=$(repo_of "$r") || return
  hook=$(pre_push_hook "$root") || deny "$hook, so it can't tell whether a pre-push hook gates this push."
  [[ -z $hook ]] || deny "$root has a pre-push hook, $hook, which \`jj git push\` never runs."
}

each_command check_push "$cmd" "$(dir_of "$(field .cwd)" "")" jj

exit 0
