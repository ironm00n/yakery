#!/usr/bin/env bash
# PermissionRequest guard for background sessions: nobody answers their prompts (redox sat 47 minutes on
# one, 11-03), so every prompt is denied with a reason the model reads, and dispatch is told what was asked.
# A background session's hooks get CLAUDE_JOB_DIR; CLAUDE_CODE_SESSION_KIND stays in the session itself.
set -uo pipefail
[[ -n ${CLAUDE_JOB_DIR:-} ]] || exit 0

payload=$(cat)
name=$(jq -r '.name // empty' "$CLAUDE_JOB_DIR/state.json" 2>/dev/null)
asked=$(jq -c '{tool_name, tool_input, permission_suggestions}' <<<"$payload" 2>/dev/null || printf '%s' "$payload")
timeout 20 "${CC_PEER:-$HOME/workspace/claude/tools/cc-peer.py}" send "$(hostname)" dispatch \
  "background session ${name:-${CLAUDE_JOB_DIR##*/}} was denied a permission prompt, since nobody answers one there: ${asked:0:2000}" \
  >/dev/null 2>&1

cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied: this call needs a permission prompt, and nobody answers one in a background session. Dispatch has been told what you asked for. Do it another way that needs no approval; if there is none, carry on with the rest of the work and name this in your report. Don't stop to wait for approval."}}}
JSON
