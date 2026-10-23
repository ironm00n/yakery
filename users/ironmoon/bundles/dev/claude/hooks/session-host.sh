#!/usr/bin/env bash
# Tells the model which machine it runs on, at session start, resume and compaction and when a
# subagent spawns: Owen works across several machines, and files, clipboards and screenshots live
# on the one he is at, which need not be this one.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

event=$(jq -r '.hook_event_name // empty' <<<"$payload")
[[ "$event" == SessionStart || "$event" == SubagentStart ]] || exit 0
read -r host </proc/sys/kernel/hostname 2>/dev/null || host=$(uname -n)
[[ -n "$host" ]] || exit 0

jq -cn --arg e "$event" --arg c "Host: this session runs on $host." \
  '{hookSpecificOutput: {hookEventName: $e, additionalContext: $c}}'
exit 0
