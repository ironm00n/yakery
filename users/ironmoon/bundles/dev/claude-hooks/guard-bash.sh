#!/usr/bin/env bash
# PreToolUse(Bash) guard: exit 2 rejects redundant forms that miss the allow-list and
# force a permission prompt — `cd <root>`, `git -C <root>`, and `echo`/`printf` banners.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
[[ -n "$cmd" ]] || exit 0

projectdir=${CLAUDE_PROJECT_DIR:-}
esc=$(printf '%s' "$projectdir" | sed -E 's/[][(){}.^$*+?|\\]/\\&/g')
sep='([[:space:]]|[;&|]|$)'

if [[ -n "$projectdir" ]] &&
  grep -Eq "(^|[;&|])[[:space:]]*cd[[:space:]]+['\"]?(${esc}|[.]|[\$][{]?CLAUDE_PROJECT_DIR[}]?)['\"]?/?${sep}" <<<"$cmd"; then
  msg="Blocked: \`cd <root> && ...\` is a compound command which unnecessarily"
  msg+=" trips up static analysis, often forcing a permission prompt."
  msg+=" The harness already runs in the project root."
  printf '%s\n' "$msg" >&2
  exit 2
fi

if [[ -n "$projectdir" ]] &&
  grep -Eq "(^|[;&|])[[:space:]]*git[[:space:]]+-C[[:space:]]+['\"]?(${esc}|[.]|[\$][{]?CLAUDE_PROJECT_DIR[}]?)['\"]?/?${sep}" <<<"$cmd"; then
  msg="Blocked: \`git -C <root>\` redundantly points at the project root and trips up"
  msg+=" static analysis, often forcing a permission prompt. The harness already runs"
  msg+=" there, so drop \`-C\` and run git directly."
  printf '%s\n' "$msg" >&2
  exit 2
fi

if grep -Eq "(^|[;&|])[[:space:]]*(echo|printf)[[:space:]][^;&|]*([=#*_~-]{3,}|[─━═]{3,})" <<<"$cmd"; then
  msg="Blocked: chaining an \`echo\`/\`printf\` banner into another command unnecessarily"
  msg+=" trips up static analysis, often forcing a permission prompt."
  msg+=" Run each command as its own tool call instead."
  printf '%s\n' "$msg" >&2
  exit 2
fi

exit 0
