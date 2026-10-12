h="${1:-$HOME/.claude/hooks/session-time.sh}"

check() {
  out=$(jq -cn --argjson p "$2" '$p' | bash "$h" |
    jq -r '.hookSpecificOutput.additionalContext // empty')
  want=$([ "$1" = EMPTY ] && echo '^$' || echo "$1")
  ok=$(grep -qE -- "$want" <<<"$out" && echo y)
  [ "$ok" = y ] && printf 'ok   %s\n' "$1" || printf 'FAIL want=%s got=%s\n' "$1" "$out"
}

start() { echo "{\"hook_event_name\":\"SessionStart\",\"source\":\"$1\"}"; }
stamp='^Current time: [A-Z][a-z]{2} [0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2} [A-Z+0-9-]+'

check "$stamp, when this prompt was sent\.$" '{"hook_event_name":"UserPromptSubmit"}'
check "$stamp, when you were spawned\.$" '{"hook_event_name":"SubagentStart"}'
check "$stamp, when this session was compacted\.$" "$(start compact)"
check EMPTY "$(start startup)"
check EMPTY "$(start resume)"
check EMPTY '{"hook_event_name":"PreToolUse"}'
