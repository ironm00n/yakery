h="${1:-$HOME/.claude/hooks/guard-kill.sh}"
dir=$(realpath "$(mktemp -d)")
mkdir -p "$dir/mine/sub" "$dir/other" "$dir/workspace/claude"
start() { (cd "$1" && exec setsid sleep 300) & }
start "$dir/mine/sub"; mine=$!
start "$dir/other"; other=$!
start "$dir/workspace/claude"; root=$!
sleep 0 & dead=$!
wait "$dead"
trap 'kill "$mine" "$other" "$root"; rm -r "${dir:?}"' EXIT

check() {
  out=$(jq -cn --arg c "$3" --arg cwd "$2" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' | HOME=$dir bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %-18s %s\n' "$got" "${2#"$dir"}" "$3" || printf 'FAIL want=%s got=%s %s  %s\n' "$1" "$got" "${2#"$dir"}" "$3"
}
me="$dir/mine"

check DENY "$me" 'pkill -f definitely-nothing-zz'
check DENY "$me" 'killall -9 sleep'
check DENY "$me" 'sudo pkill sleep'
check DENY "$me" 'env -u command -v pkill -f definitely-nothing-zz'
check PASS "$me" 'command -v pkill && echo found'
check DENY "$me" '/run/current-system/sw/bin/pkill sleep'
check DENY "$me" "bash -c 'pkill sleep'"
check DENY "$me" 'kill $(pgrep -f "measure.py|cargo test --workspace")'
check DENY "$me" 'kill -9 `pidof sleep`'
check DENY "$me" 'pgrep -af measure.py | xargs kill'
check DENY "$me" 'pgrep sleep | xargs -r -n 1 kill -TERM'
check DENY "$me" 'fuser -k 8080/tcp'
check DENY "$me" 'kill "$pid"'

check DENY "$me" "kill $other"
check DENY "$me" "kill -9 $other"
check DENY "$me" "kill -s TERM $mine $other"
check DENY "$me" "kill -- -$other"
check DENY "$dir/workspace/claude" "kill $root"
check DENY "$dir" "kill $mine"
check DENY / "kill $mine"

check PASS "$me" "kill $mine"
check PASS "$me" "kill -TERM $mine 2>/dev/null"
check PASS "$me" "kill -- -$mine"
check PASS "$dir/other" "kill $other"
check PASS "$me" "kill -0 $other"
check PASS "$me" "kill $dead"
check PASS / "kill -9 $dead"
check PASS "$dir" 'sleep 300 & kill $!'
check PASS "$dir" 'sleep 300 & p=$!; kill "$p"'
check PASS "$dir" 'kill -l'

check PASS "$me" 'rg -n pkill src/'
check PASS "$me" 'git commit -m "refuse pkill; killall and kill $(pgrep x) too"'
check PASS "$me" 'pgrep -af measure.py'
check PASS "$me" 'cargo test --workspace'
check PASS "$me" 'cat <<EOF > notes.md
pkill -f measure.py
EOF'

root_cwd="$dir/workspace/claude"
check PASS "$root_cwd" "KILL_GUARD_OVERRIDE='leaked test daemon, Owen approved' kill $other"
check PASS "$root_cwd" "KILL_GUARD_OVERRIDE=leak kill -9 $other $dead"
check PASS "$root_cwd" "env KILL_GUARD_OVERRIDE=leak kill $other"
check DENY "$root_cwd" "KILL_GUARD_OVERRIDE= kill $other"
check DENY "$root_cwd" "KILL_GUARD_OVERRIDE='' kill $other"
check DENY "$root_cwd" "KILL_GUARD_OVERRIDE=leak true; kill $other"
check DENY "$me" 'KILL_GUARD_OVERRIDE=leak pkill sleep'
check DENY "$me" 'KILL_GUARD_OVERRIDE=leak kill $(pgrep sleep)'
check DENY "$me" 'pgrep sleep | KILL_GUARD_OVERRIDE=leak xargs kill'
check DENY "$me" 'KILL_GUARD_OVERRIDE=leak fuser -k 8080/tcp'
check DENY "$me" 'KILL_GUARD_OVERRIDE=leak kill "$pid"'

log="$dir/.local/state/claude/kill-overrides.log"
grep -q "reason leaked test daemon, Owen approved" "$log" 2>/dev/null && grep -q "pid $other	sleep 300" "$log" &&
  echo "ok   override log names the reason and the pid's command line" ||
  echo "FAIL override log: $(cat "$log" 2>/dev/null || echo missing)"

# a reader that fails blocks the call (a PreToolUse hook's exit 2) instead of letting the line through
mkdir -p "$dir/broken" && printf '#!/bin/sh\nexit 2\n' >"$dir/broken/awk" && chmod +x "$dir/broken/awk"
jq -cn --arg c "kill $mine" --arg cwd "$me" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' |
  PATH="$dir/broken:$PATH" HOME=$dir bash "$h" >/dev/null 2>&1
[ $? = 2 ] && echo "ok   a reader that fails exits 2" || echo "FAIL a reader that fails: want exit 2"
