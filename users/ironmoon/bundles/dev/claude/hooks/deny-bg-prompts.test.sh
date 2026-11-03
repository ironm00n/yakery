h="${1:-$HOME/.claude/hooks/deny-bg-prompts.sh}"
dir=$(mktemp -d)
mkdir "$dir/job" && echo '{"name":"redox"}' > "$dir/job/state.json"
printf '#!/bin/sh\nprintf "%%s\\n" "$*" > %s/sent\n' "$dir" > "$dir/peer" && chmod +x "$dir/peer"
check() {
  rm -f "$dir/sent"
  out=$(jq -cn --arg c 'cp a ~/.claude/jobs/x/tmp/b' '{tool_name:"Bash", tool_input:{command:$c}}' |
    env -u CLAUDE_JOB_DIR ${2:+CLAUDE_JOB_DIR=$2} CC_PEER="$dir/peer" bash "$h")
  got=$(jq -r '.hookSpecificOutput.decision.behavior // "pass"' <<<"${out:-{\}}")
  sent=$([ -s "$dir/sent" ] && rg -q 'dispatch background session redox .*"tool_name":"Bash"' "$dir/sent" && echo told || echo silent)
  [ "$got $sent" = "$1" ] && printf 'ok   %-12s job=%s\n' "$1" "${2:-unset}" || printf 'FAIL want=%s got=%s %s  job=%s\n' "$1" "$got" "$sent" "${2:-unset}"
}

check 'deny told' "$dir/job"
check 'pass silent' ''
rm -r "$dir"
