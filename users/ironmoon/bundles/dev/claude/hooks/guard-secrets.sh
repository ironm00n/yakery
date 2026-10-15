#!/usr/bin/env bash
# PreToolUse guard for ~/.claude/secrets, whose keys are used only inline as `$(<path)` and never
# shown. Denies Read/Edit/Write/Grep/Glob inside the directory, and Bash that names it any other way
# or hands an inline key to echo/printf. Catches slips, not intent: a path built at runtime passes.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

secrets=$(realpath -m "$HOME/.claude/secrets")

deny() {
  local r="Blocked: $1. Keys in ~/.claude/secrets are used only inline, as"
  r+=" \`\$(<~/.claude/secrets/<name>)\` inside the command that needs them, and never printed."
  jq -cn --arg r "$r" '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

tool=$(jq -r '.tool_name // ""' <<<"$payload")

if [[ $tool == Bash ]]; then
  cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
  [[ $cmd == *.claude/secrets* ]] || exit 0
  inline='\$\(<[[:space:]]*(~|\$HOME|\$\{HOME\}|/home/[^/]+)/\.claude/secrets/[[:alnum:]_.-]+\)'
  grep -Eq "(^|[;&|(])[[:space:]]*(echo|printf)[^;&|]*${inline}" <<<"$cmd" &&
    deny "\`echo\`/\`printf\` would print the key"
  [[ $(sed -E "s#${inline}##g" <<<"$cmd") == *.claude/secrets* ]] &&
    deny "this names ~/.claude/secrets outside an inline \`\$(<...)\`"
  exit 0
fi

cwd=$(jq -r '.cwd // empty' <<<"$payload")
while IFS= read -r p; do
  [[ $p == \~ || $p == \~/* ]] && p=$HOME${p:1}
  [[ $p == /* ]] || p=${cwd:-$PWD}/$p
  p=$(realpath -m "$p")
  [[ $p == "$secrets" || $p == "$secrets"/* ]] && deny "$tool would open $p"
done < <(jq -r 'if .tool_name == "Grep" then .tool_input.path
                else .tool_input | (.file_path, .notebook_path, .path, .pattern) end | values' <<<"$payload")

exit 0
