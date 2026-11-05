#!/usr/bin/env bash
# PreToolUse(SendMessage) guard: the lane named chat only talks with Owen and is never sent messages, so it can keep to
# what he asks of it (Owen 11-05 17:13). It has no mail topic, so a cross-session message is the one way to reach it:
# one addressed to it by name, bare or with its [ref], is refused, from every session. Its socket path is no address a
# session learns while chat sends nothing.
set -uo pipefail
command -v jq >/dev/null 2>&1 || exit 0
to=$(jq -r '.tool_input.to // ""' 2>/dev/null) || exit 0
shopt -s nocasematch
[[ $to =~ ^[[:space:]]*chat([[:space:]]*\[[^]]*\])?[[:space:]]*$ ]] || exit 0
jq -cn '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:"Refused: the chat lane only talks with Owen, and no session sends it messages (Owen 11-05 17:13). Tell dispatch, or mail Owen, instead."}}'
