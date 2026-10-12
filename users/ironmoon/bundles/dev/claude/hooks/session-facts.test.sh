h="${1:-$HOME/.claude/hooks/session-facts.sh}"
TMP=$(mktemp -d)
trap 'rm -rf -- "$TMP"' EXIT
export XDG_RUNTIME_DIR="$TMP/run"
T="$TMP/s1.jsonl"
mkdir -p "$TMP/s1/subagents"
: >"$T"

check() {
  out=$(jq -cn --argjson p "$2" '{session_id: "s1", transcript_path: $t} + $p' --arg t "$T" | bash "$h" |
    jq -r '.hookSpecificOutput.additionalContext // empty')
  if [ "$1" = EMPTY ]; then ok=$([ -z "$out" ] && echo y); else ok=$(grep -qF -- "$1" <<<"$out" && echo y); fi
  [ "$ok" = y ] && printf 'ok   %s\n' "$1" || printf 'FAIL want=%s got=%s\n' "$1" "$out"
}

check 'harness claude-code' '{"hook_event_name":"SessionStart","source":"startup"}'
check EMPTY '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'

# The real entry sits under a synthetic one and an agent-progress entry nesting another model.
cat >>"$T" <<'EOF'
{"type":"assistant","message":{"model":"claude-opus-5-5"}}
{"type":"assistant","message":{"model":"<synthetic>"}}
{"type":"progress","data":{"message":{"type":"assistant","message":{"model":"claude-haiku-5-5"}}}}
EOF
check 'trailer root `claude-opus-5-5:xhigh claude-code' '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'
check EMPTY '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'
check 'now `claude-opus-5-5:max' '{"hook_event_name":"PreToolUse","effort":{"level":"max"}}'
check 'was `claude-opus-5-5:max' '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'

check 'from claude-opus-5-5 to claude-fable-5-1; trailer root `claude-fable-5-1:xhigh' \
  '{"hook_event_name":"PostModelSwitch","from_model":"claude-opus-5-5","to_model":"claude-fable-5-1"}'
# The transcript still names the old model until the new one answers.
check EMPTY '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'

check 'spawned you has trailer root `claude-fable-5-1:xhigh' '{"hook_event_name":"SubagentStart","agent_id":"a1"}'
echo '{"type":"assistant","message":{"model":"claude-haiku-5-5"}}' >"$TMP/s1/subagents/agent-a1.jsonl"
check 'you run `claude-haiku-5-5:low`, under the trailer root `claude-fable-5-1:xhigh' \
  '{"hook_event_name":"PreToolUse","agent_id":"a1","effort":{"level":"low"}}'
check EMPTY '{"hook_event_name":"PreToolUse","agent_id":"a1","effort":{"level":"low"}}'

check 'model claude-opus-5-5' '{"hook_event_name":"SessionStart","source":"compact"}'
check 'trailer root `claude-opus-5-5:xhigh' '{"hook_event_name":"PreToolUse","effort":{"level":"xhigh"}}'

# A switch before the first tool call leaves effort unknown; filling it in is not a change.
rm -rf "$XDG_RUNTIME_DIR"
check 'trailer root `claude-fable-5-1 claude-code' \
  '{"hook_event_name":"PostModelSwitch","from_model":"claude-opus-5-5","to_model":"claude-fable-5-1"}'
check 'trailer root `claude-fable-5-1:high' '{"hook_event_name":"PreToolUse","effort":{"level":"high"}}'

# A lane whose transcript entry lands only after its first PreToolUse is caught by PostToolUse.
check EMPTY '{"hook_event_name":"PreToolUse","agent_id":"a2","effort":{"level":"medium"}}'
echo '{"type":"assistant","message":{"model":"claude-opus-5-5"}}' >"$TMP/s1/subagents/agent-a2.jsonl"
check 'you run `claude-opus-5-5:medium`' '{"hook_event_name":"PostToolUse","agent_id":"a2","effort":{"level":"medium"}}'

rm -rf "$XDG_RUNTIME_DIR"
check 'Provenance facts: your own model' '{"hook_event_name":"SubagentStart","agent_id":"a3"}'
