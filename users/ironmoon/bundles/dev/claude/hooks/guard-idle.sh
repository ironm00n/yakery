#!/usr/bin/env bash
# PreToolUse(Bash) guard: denies commands whose only effect is to spend wall-clock time — long
# sleeps, loops that poll `date`, `at`/systemd self-wakeups. A timed /goal is a floor on work, so
# reaching the end of the window by waiting is circumvention rather than compliance. autoMode's
# soft_deny says as much, but the classifier only runs in auto mode; this is deterministic and
# mode-independent. Short sleeps pass — waiting a few seconds on a process that is coming up is
# real work. Total sleep summed inside a loop is not accounted for (undecidable in general); the
# date-polling and `while true` shapes stand in for it.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

MAX_SLEEP_SECS=60

deny() {
  local advice="A /goal window is satisfied by work, not by elapsed time: re-trace the last change,"
  advice+=" re-derive a claim from primary source, attack the weakest artifact, or report what is"
  advice+=" left and its expected value so it can be redirected."
  jq -cn --arg r "$1 $advice" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
[[ -n "$cmd" ]] || exit 0

# A loop whose condition reads the clock is a timer however short its sleep.
if grep -Eq '(^|[;&|[:space:]])(until|while)[[:space:]][^;]*\bdate\b' <<<"$cmd"; then
  deny 'Blocked: a loop whose condition polls `date` is a wall-clock timer.'
fi

if grep -Eq '(^|[;&|[:space:]])while[[:space:]]+(true|:)[[:space:]]*;?[[:space:]]*do[^;]*\bsleep\b' <<<"$cmd"; then
  deny 'Blocked: `while true; do sleep ...` has no exit condition but elapsed time.'
fi

# Command position only, never merely after a space: `at` is too common a word, and commands
# carry prose in heredocs — "cost at 12:00" tripped this rule on its first real use.
if grep -Eq '(^|[;&|])[[:space:]]*(at|batch)[[:space:]]+([0-9]{1,2}:[0-9]|[0-9]{1,4}(am|pm)|now|noon|midnight|teatime|tomorrow)' <<<"$cmd"; then
  deny 'Blocked: `at` schedules a wake-up instead of doing the work now.'
fi

if grep -Eq 'systemd-run[^;]*--on-(active|calendar|unit-active|boot)' <<<"$cmd"; then
  deny 'Blocked: a systemd timer schedules a wake-up instead of doing the work now.'
fi

while read -r dur; do
  [[ -n "$dur" ]] || continue
  secs=$(awk -v d="$dur" 'BEGIN {
    n = d + 0; u = d; sub(/^[0-9.]+/, "", u)
    if (u == "m") n *= 60; else if (u == "h") n *= 3600; else if (u == "d") n *= 86400
    printf "%d", n
  }')
  if ((secs >= MAX_SLEEP_SECS)); then
    deny "Blocked: \`sleep $dur\` waits ${secs}s; anything at or over ${MAX_SLEEP_SECS}s is idling, not waiting on a process."
  fi
done < <(grep -Eo '\bsleep[[:space:]]+[0-9]+([.][0-9]+)?[smhd]?' <<<"$cmd" |
  grep -Eo '[0-9]+([.][0-9]+)?[smhd]?$')

exit 0
