#!/usr/bin/env bash
# PreToolUse(Bash) guard: a session kills only processes it can show are its own. Pattern kills are
# refused, since other lanes' jobs match the same patterns: on 11-03 a lane's `pgrep -af ... ; kill`
# took another lane's 6-minute gate test (friction 96). A literal PID passes when its /proc cwd is
# inside the caller's cwd and that cwd is not a root other sessions share; a dead PID passes.
# `KILL_GUARD_OVERRIDE='<reason>' kill <pid>...` skips the ownership check for literal PIDs only
# and is logged (Owen 11-04 01:06: "change the guard so you can override with with an env var if needed").

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
[[ $cmd =~ kill|fuser ]] || exit 0
cwd=$(realpath -m "$(jq -r '.cwd // empty' <<<"$payload")" 2>/dev/null) || cwd=/

own="Kill only PIDs you started: note \`\$!\` when you start the job and \`kill <that pid>\` from inside"
own+=" your own worktree, or stop your own background task with TaskStop."

deny() {
  jq -cn --arg r "Blocked: $1 $own" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}
by_pattern() { deny "\`$1\` picks processes by pattern, and other sessions' processes match it too."; }

mapfile -t shared < <(realpath -m / "$HOME" "$HOME/workspace/redox" "$HOME/workspace/claude" "$HOME"/research/* /etc/nixos "$HOME/tmp/worktree" /tmp)
from_bang=" $(grep -Eo '\b[A-Za-z_][A-Za-z0-9_]*=\$!' <<<"$cmd" | sed 's/=.*//' | tr '\n' ' ')"

check_kill() {
  local a i sig= dd= pids=() p t r
  local -a args=("$@")
  for ((i = 0; i < ${#args[@]}; i++)); do
    a=${args[i]}
    if [[ -z $dd ]]; then
      case $a in
        --) dd=1; continue ;;
        -l | -L | --list* | --table) return ;;
        -s | -n | --signal) ((i++)); sig=${args[i]:-}; continue ;;
        -*) sig=${a#-}; continue ;;
      esac
    fi
    case $a in
      +([0-9])) pids+=("$a") ;;
      -+([0-9])) mapfile -t -O "${#pids[@]}" pids < <(pgrep -g "${a#-}") ;;
      %* | '$!') ;;
      \$+([A-Za-z0-9_]) | \$\{+([A-Za-z0-9_])\})
        r=${a//[\$\{\}]/}
        [[ $from_bang == *" $r "* ]] || deny "\`kill $a\`: the hook can't see which process \$$r names." ;;
      '$') deny "\`kill \$(...)\` takes its PIDs from a command, and pgrep or pidof there match other sessions' processes too." ;;
      *) deny "\`kill $a\` is not a literal PID, so the hook can't see whose process it is." ;;
    esac
  done
  [[ $sig == 0 || $sig == SIG0 || $sig == NULL ]] && return
  [[ -n $override ]] && { overridden "${pids[@]}"; return; }
  for p in "${pids[@]}"; do
    [[ -e /proc/$p ]] || continue
    for r in "${shared[@]}"; do
      [[ $r == "$cwd" || $r == "$cwd"/* ]] &&
        deny "this session's cwd $cwd is shared with other sessions, so a PID's cwd can't show it is yours; cd into your own worktree first."
    done
    t=$(readlink "/proc/$p/cwd") || deny "PID $p's cwd is unreadable, so the hook can't see whose process it is."
    [[ $t == "$cwd" || $t == "$cwd"/* ]] ||
      deny "PID $p runs in $t, outside this session's cwd $cwd, so it belongs to another session."
  done
}

overridden() {
  local reason log=$HOME/.local/state/claude/kill-overrides.log p
  reason=$(grep -oE "KILL_GUARD_OVERRIDE=('[^']*'|\"[^\"]*\"|[^[:space:];&|]*)" <<<"$cmd" | head -1)
  reason=${reason#KILL_GUARD_OVERRIDE=}
  reason=${reason//[\'\"]/}
  [[ -n ${reason//[[:space:]]/} ]] || deny "KILL_GUARD_OVERRIDE needs a reason."
  mkdir -p "${log%/*}"
  {
    printf '%s\tsession %s\tcwd %s\treason %s\n' "$(date -Is)" "$(jq -r '.session_id // "-"' <<<"$payload")" "$cwd" "$reason"
    for p in "$@"; do printf '\tpid %s\t%s\n' "$p" "$(tr '\0' ' ' 2>/dev/null <"/proc/$p/cmdline" || echo '(gone)')"; done
  } >>"$log"
}

while read -ra w; do
  lead=("${w[@]}")
  command_word 'kill|pkill|killall'
  ((${#w[@]})) || continue
  override=
  for a in "${lead[@]:0:${#lead[@]}-${#w[@]}}"; do [[ $a == KILL_GUARD_OVERRIDE=?* ]] && override=1; done
  case ${w[0]##*/} in
    pkill | killall | skill) by_pattern "${w[0]##*/}" ;;
    fuser) [[ " ${w[*]:1} " =~ \ -[[:alnum:]]*k ]] && deny "\`fuser -k\` kills every process using a file or port, whoever started it." ;;
    kill)
      [[ -n $fed ]] && by_pattern "xargs kill"
      check_kill "${w[@]:1}" ;;
  esac
done < <(segments "$cmd")

exit 0
