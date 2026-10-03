#!/usr/bin/env bash
# PermissionRequest(Bash) guard: denies redundant forms that would miss the allow-list and
# force a prompt — `cd <cwd> &&`, `git -C <cwd>`, `echo`/`printf` banners. "Redundant" = the
# target is the cwd the shell already sits in; a `cd` to a different dir (e.g. project root from
# a subdir) is a real move and passes. Deny is a JSON decision, not `exit 2` (ignored here).

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

deny() {
  jq -cn --arg m "$1" '{hookSpecificOutput:{hookEventName:"PermissionRequest",decision:{behavior:"deny",message:$m}}}'
  exit 0
}

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
[[ -n "$cmd" ]] || exit 0

cwd=$(jq -r '.cwd // ""' <<<"$payload")
[[ -n "$cwd" ]] || cwd=${CLAUDE_PROJECT_DIR:-}
esc=$(printf '%s' "$cwd" | sed -E 's/[][(){}.^$*+?|\\]/\\&/g')
sep='([[:space:]]|[;&|]|$)'

# A cd/-C target is redundant only when it resolves to the cwd the shell already sits in:
# the literal cwd, ".", or a $CLAUDE_PROJECT_DIR reference when the project root *is* the cwd.
alt="${esc}|[.]"
[[ "${CLAUDE_PROJECT_DIR:-}" == "$cwd" ]] && alt="${alt}|[\$][{]?CLAUDE_PROJECT_DIR[}]?"

if [[ -n "$cwd" ]] &&
  grep -Eq "(^|[;&|])[[:space:]]*cd[[:space:]]+['\"]?(${alt})['\"]?/?[[:space:]]*[;&|]" <<<"$cmd"; then
  msg="Blocked: \`cd <cwd> && ...\` is a no-op (the shell is already in that dir) that only"
  msg+=" trips static analysis into a permission prompt. Drop the \`cd\`."
  deny "$msg"
fi

if [[ -n "$cwd" ]] &&
  grep -Eq "(^|[;&|])[[:space:]]*git[[:space:]]+-C[[:space:]]+['\"]?(${alt})['\"]?/?${sep}" <<<"$cmd"; then
  msg="Blocked: \`git -C <cwd>\` points at the dir the shell is already in, only tripping"
  msg+=" static analysis into a permission prompt. Drop \`-C\` and run git directly."
  deny "$msg"
fi

if grep -Eq "(^|[;&|])[[:space:]]*(echo|printf)[[:space:]][^;&|]*([=#*_~-]{3,}|[─━═]{3,})" <<<"$cmd"; then
  msg="Blocked: chaining an \`echo\`/\`printf\` banner into another command unnecessarily"
  msg+=" trips up static analysis, often forcing a permission prompt."
  msg+=" Run each command as its own tool call instead."
  deny "$msg"
fi

exit 0
