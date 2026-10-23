h="${1:-$HOME/.claude/hooks/session-host.sh}"
host=$(</proc/sys/kernel/hostname)

check() {
  out=$(jq -cn --argjson p "$2" '$p' | bash "$h" |
    jq -r '.hookSpecificOutput.additionalContext // empty')
  want=$([ "$1" = EMPTY ] || echo "$1")
  ok=$([ "$out" = "$want" ] && echo y)
  [ "$ok" = y ] && printf 'ok   %s\n' "$1" || printf 'FAIL want=%s got=%s\n' "$1" "$out"
}

check "Host: this session runs on $host." '{"hook_event_name":"SessionStart","source":"startup"}'
check "Host: this session runs on $host." '{"hook_event_name":"SessionStart","source":"compact"}'
check "Host: this session runs on $host." '{"hook_event_name":"SubagentStart","agent_id":"a1"}'
check EMPTY '{"hook_event_name":"UserPromptSubmit"}'
check EMPTY '{"hook_event_name":"PreToolUse"}'
