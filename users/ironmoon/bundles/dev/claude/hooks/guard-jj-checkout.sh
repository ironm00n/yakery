#!/usr/bin/env bash
# PreToolUse(Bash) guard: jj snapshots its working copy before every command, even one that then
# fails, so a jj run in one of Owen's own checkouts writes whatever sits on disk there into his @.
# On 11-04 a subagent's cwd reset to ~/workspace/redox and a `jj file show -r <another workspace's
# change>` amended his @ with 65 files (friction 98). A jj whose repo is one of his checkouts passes
# only with --ignore-working-copy. The one exception is the main session started in
# ~/workspace/claude, in the foreground or as the job named dispatch, on that checkout for its rebases onto master and on /etc/nixos for
# the moves before his switch (his 10-18 and 10-29 rules).

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"
. "${BASH_SOURCE[0]%/*}/lib/jj.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd == *jj* ]] || exit 0

mapfile -t checkouts < <(realpath -m "$HOME/workspace/redox" "$HOME/workspace/claude" /etc/nixos "$HOME"/research/*)
is_dispatch() {
  [[ -z $(field .agent_id) &&
    $(field .transcript_path) == "$HOME/.claude/projects/${HOME//[^[:alnum:]]/-}-workspace-claude/"* ]] || return
  [[ -z ${CLAUDE_JOB_DIR-} ]] || [[ $(jq -r .name "$CLAUDE_JOB_DIR/state.json" 2>/dev/null) == dispatch ]]
}
dispatch=()
is_dispatch && mapfile -t dispatch < <(realpath -m "$HOME/workspace/claude" /etc/nixos)

instead="Run it against your own jj workspace with \`-R <your workspace>\`, or add \`--ignore-working-copy\` for a read."
deny() {
  jq -cn --arg r "Blocked: $1 $instead" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

check_jj() {
  [[ ${w[0]##*/} == jj ]] || return
  set -- "${w[@]:1}"
  local a i r=$here repo c d
  for ((i = 1; i <= $#; i++)); do
    a=${!i}
    case $a in
      --) break ;;
      --ignore-working-copy) return ;;
      -R | --repository) ((i++)); r=$(dir_of "${!i-}" "$here") ;;
      --repository=*) r=$(dir_of "${a#*=}" "$here") ;;
      -R?*) r=$(dir_of "${a#-R}" "$here") ;;
    esac
  done
  [[ -n $r ]] || deny "the hook can't read off the line which repo this \`jj\` runs in (a variable, substitution, quoted path with blanks or \`cd -\`), so it may be one of Owen's checkouts."
  repo=$(repo_of "$r") || return
  for d in "${dispatch[@]}"; do [[ $repo == "$d" ]] && return; done
  for c in "${checkouts[@]}"; do
    [[ $repo == "$c" ]] &&
      deny "this \`jj\` runs in $repo, Owen's own checkout, and jj snapshots the files on disk into his @ before any command, even one that then fails."
  done
}

each_command check_jj "$cmd" "$(dir_of "$(field .cwd)" "")" jj

exit 0
