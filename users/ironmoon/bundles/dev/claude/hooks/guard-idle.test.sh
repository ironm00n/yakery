h="${1:-$HOME/.claude/hooks/guard-idle.sh}"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
goal() { jq -cn --argjson met "$1" '{type:"attachment",attachment:{type:"goal_status",condition:"finish the port",met:$met}}'; }
{ goal false; echo '{"type":"user","message":{"content":"go"}}'; } >"$dir/running"
{ goal false; goal true; } >"$dir/met"
echo '{"type":"user","message":{"content":"no goal here"}}' >"$dir/none"
jq -cn '{type:"user",message:{content:[{type:"tool_result",content:"{\"type\":\"goal_status\",\"met\":false}"}]}}' >"$dir/quoted"

check() {
  out=$(jq -cn --arg c "$3" --arg t "$2" '{tool_name:"Bash",tool_input:{command:$c},transcript_path:$t}' | bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %-7s %s\n' "$got" "${2##*/}" "$3" || printf 'FAIL want=%s got=%s %s  %s\n' "$1" "$got" "${2##*/}" "$3"
}
run="$dir/running"

check DENY "$run" 'until [ "$(date +%H%M)" -ge "1000" ]; do sleep 120; done; echo reached'
check DENY "$run" 'while [ $(date +%s) -lt 1600000000 ]; do sleep 1; done'
check DENY "$run" 'while true; do sleep 5; done'
check DENY "$run" 'sleep 3600'
check DENY "$run" 'sleep 2m'
check DENY "$run" 'sleep 1.5h'
check DENY "$run" 'sleep 60'
check DENY "$run" 'at 10:00 <<< "echo hi"'
check DENY "$run" 'echo hi; at noon'
check DENY "$run" 'systemd-run --user --on-active=3h /bin/true'

check PASS "$run" 'sleep 2; curl -s localhost:8080/health'
check PASS "$run" 'sleep 0.5'
check PASS "$run" 'sleep 59'
check PASS "$run" 'until curl -sf localhost:8080/health; do sleep 2; done'
check PASS "$run" 'cargo test'
check PASS "$run" 'date'
check PASS "$run" 'rg -n sleep src/'
check PASS "$run" 'git commit -m "fix the batch 3 regression"'
check PASS "$run" 'cat f.md'

# The regression that motivated the command-position fix: prose in a heredoc body.
check PASS "$run" 'cat <<EOF >> DECISIONS.md
the cost at 12:00 is ten minutes, and the two documents meet at 11:45
EOF'

# Outside a running /goal, waiting is allowed.
check PASS "$dir/none" 'sleep 3600'
check PASS "$dir/met" 'sleep 3600'
check PASS "$dir/absent" 'sleep 3600'
check PASS "$dir/quoted" 'sleep 3600'
