h="${1:-$HOME/.claude/hooks/guard-idle.sh}"
check() {
  out=$(jq -cn --arg c "$2" '{tool_name:"Bash",tool_input:{command:$c}}' | bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %s\n' "$got" "$2" || printf 'FAIL want=%s got=%s  %s\n' "$1" "$got" "$2"
}

check DENY 'until [ "$(date +%H%M)" -ge "1000" ]; do sleep 120; done; echo reached'
check DENY 'while [ $(date +%s) -lt 1600000000 ]; do sleep 1; done'
check DENY 'while true; do sleep 5; done'
check DENY 'sleep 3600'
check DENY 'sleep 2m'
check DENY 'sleep 1.5h'
check DENY 'sleep 60'
check DENY 'at 10:00 <<< "echo hi"'
check DENY 'echo hi; at noon'
check DENY 'systemd-run --user --on-active=3h /bin/true'

check PASS 'sleep 2; curl -s localhost:8080/health'
check PASS 'sleep 0.5'
check PASS 'sleep 59'
check PASS 'until curl -sf localhost:8080/health; do sleep 2; done'
check PASS 'cargo test'
check PASS 'date'
check PASS 'rg -n sleep src/'
check PASS 'git commit -m "fix the batch 3 regression"'
check PASS 'cat f.md'

# The regression that motivated the command-position fix: prose in a heredoc body.
check PASS 'cat <<EOF >> DECISIONS.md
the cost at 12:00 is ten minutes, and the two documents meet at 11:45
EOF'
