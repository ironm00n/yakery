#!/usr/bin/env bash
# Tells the model the local time when a prompt arrives, a subagent spawns, or a session compacts
# (which can happen mid-turn, with no prompt after it). The note repeats CLAUDE.md's rule on using
# the time, because sessions paced their work by the hour while the rule was only there.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

event=$(jq -r '.hook_event_name // empty' <<<"$payload")
case "$event" in
UserPromptSubmit) when="when this prompt was sent" ;;
SubagentStart) when="when you were spawned" ;;
SessionStart)
  [[ "$(jq -r '.source // empty' <<<"$payload")" == compact ]] || exit 0
  when="when this session was compacted"
  ;;
*) exit 0 ;;
esac

rule="For dating only, never a reason to hurry, cut work or remark on the hour."

jq -cn --arg e "$event" --arg c "Current time: $(date '+%a %Y-%m-%d %H:%M %Z'), $when. $rule" \
  '{hookSpecificOutput: {hookEventName: $e, additionalContext: $c}}'
exit 0
