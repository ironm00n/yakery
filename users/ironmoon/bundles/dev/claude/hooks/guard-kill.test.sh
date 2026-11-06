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
  out=$(jq -cn --arg c "$3" --arg cwd "$2" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' | HOME=$dir bash "$h" 2>/dev/null)
  rc=$?
  got=$( ((rc)) && echo "EXIT$rc" || { [ -n "$out" ] && echo DENY || echo PASS; })
  [ "$got" = "$1" ] && printf 'ok   %-4s %-18s %s\n' "$got" "${2#"$dir"}" "$3" || printf 'FAIL want=%s got=%s %s  %s\n' "$1" "$got" "${2#"$dir"}" "$3"
}
# the deny reason holds $3
says() {
  out=$(jq -cn --arg c "$2" --arg cwd "$1" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' | HOME=$dir bash "$h" 2>/dev/null |
    jq -r '.hookSpecificOutput.permissionDecisionReason')
  [[ $out == *"$3"* ]] && printf 'ok   says %-18s %s\n' "${1#"$dir"}" "$2" || printf 'FAIL says %s: %s  %s\n' "$3" "${1#"$dir"}" "$2"
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

# friction 62: a pgrep whose pattern finds the shell running the line, on a line with a loop or a wait on a PID
for line in "until ! pgrep -f 'fig1-test3.log'; do sleep 5; done" "wait-gone \$(pgrep -f 'compare-finish-job19' | head -1)" \
  'wait-gone --timeout 60 `pgrep -f foo`' 'while pgrep -f foo >/dev/null; do sleep 5; done' \
  'while [ -n "$(pgrep --full foo)" ]; do sleep 5; done' 'while :; do pgrep -af foo || break; sleep 5; done' \
  "zsh -c \"until ! pgrep -f 'cargo test'; do sleep 5; done\"" "until ! pgrep -f 'rows27.py /home'; do sleep 30; done" \
  "until ! pgrep -f 'rows27.py /home' >/dev/null; do sleep 30; done; tail -5 rows27.log" \
  "tail fig1.log; until ! pgrep -f '[f]ig1.log'; do sleep 5; done" \
  "true qqcargo test & until ! pgrep -f '[q]qcargo.*test' >/dev/null; do sleep 1; done" \
  "echo qqjob7_log; until ! pgrep -f '[q]qjob7.log'; do sleep 1; done" "cat QQJOB7.md; until ! pgrep -if '[q]qjob7'; do sleep 1; done" \
  'pidwait -f qqjob7' 'pidwait -A -f qqjob7' 'tail --pid=$(pgrep -of qqjob7) -f /dev/null' 'until ! pgrep --fu qqjob7; do sleep 1; done' \
  "until ! pgrep -f '[N]O_UNSET'; do sleep 1; done" "until ! pgrep -u ironmoon -f qqjob7; do sleep 1; done" \
  'until ! pgrep -Af qqjob7 >/dev/null; do sleep 1; done' 'until ! pgrep -f --ignore-ancestors qqjob7; do sleep 1; done' \
  'while (( $(pgrep -cf qqjob7) )); do sleep 1; done' 'while :; do if ! pgrep -f qqz3r >/dev/null; then break; fi; sleep 1; done' \
  'while :; do [ -z "$(pgrep -f qqz3r)" ] && break; sleep 1; done' 'while :; do pgrep -f qqz3r >/dev/null || { break; }; sleep 1; done' \
  'while :; do pgrep -f qqz3r &>/dev/null || break; sleep 1; done' 'until false; do pgrep -f qqz3r || break; sleep 1; done' \
  $'while\n  pgrep -f qqjob7 >/dev/null\ndo sleep 1; done' "until ! pgrep -fx '.*qqjob7.*' >/dev/null; do sleep 1; done" \
  'python3 ~/workspace/claude/tools/wait-gone $(pgrep -f qqjob7)' \
  'tail --pid=$(pgrep --oldest -f qqjob7) -f /dev/null' 'while pgrep --oldest --full qqjob7 >/dev/null; do sleep 1; done' \
  'while sleep 1; do pgrep -f qqjob7 >/dev/null || break; done' 'while sleep 1; do [ -n "$(pgrep -f qqjob7)" ] || break; done' \
  'for ((;;)); do pgrep -f qqjob7 >/dev/null || break; sleep 1; done' 'while ((1)); do pgrep -f qqjob7 >/dev/null || break; done' \
  'while [ 1 ]; do pgrep -f qqjob7 >/dev/null || break; sleep 1; done' 'while (true); do pgrep -f qqjob7 || break; done' \
  'until pgrep -f qqjob7 >/dev/null; do sleep 1; done' 'while :; do pgrep -f qqjob7 | grep -q . || break; sleep 1; done' \
  'while :; do (( $(pgrep -cf qqjob7) == 0 )) && break; sleep 1; done' 'while pgrep -f qqjob7 | grep -v pgrep; do sleep 1; done' \
  $'cd /tmp/qqk4\nuntil ! pgrep -f "[q]qk4 until" >/dev/null; do sleep 1; done' \
  'while \pgrep -f qqjob7 >/dev/null; do sleep 1; done' 'while nice -n 10 pgrep -f qqjob7 >/dev/null; do sleep 1; done' \
  "eval 'while pgrep -f qqjob7 >/dev/null; do sleep 1; done'" "flock /tmp/qqf -c 'until ! pgrep -f qqjob7 >/dev/null; do sleep 1; done'" \
  "bash -c 'until ! pgrep -f '\\''qq job'\\''; do :; done'" 'until ! pgrep -f "$pat" >/dev/null; do sleep 1; done' \
  'while read -r l; do :; done < <(pgrep -af qqjob7); wait-gone $pid' \
  "until [ -z \"\`pgrep -f 'qqjob7'\`\" ]; do sleep 1; done"; do
  check DENY "$me" "$line"
done
# the scoped review of 11-06 14:3x: bounded and zsh loops, functions, a quote in a comment or heredoc, more waits,
# a quoted name, a second backtick span, an inner shell, the wrapper's own text, `~`
for line in 'for i in $(seq 600); do pgrep -f qqzr41 >/dev/null || break; sleep 1; done; echo ended-$i' \
  'for i in {1..600}; do pgrep -f qqzr42 >/dev/null || break; sleep 1; done' \
  'for i ({1..600}) { pgrep -f qqzr73 >/dev/null || break; sleep 1 }' \
  $'foreach i ({1..600})\npgrep -f qqzr56 >/dev/null || break; sleep 1\nend' \
  'repeat 600 do pgrep -f qqzr43 >/dev/null || break; sleep 1; done' 'w() { pgrep -f qqzr72 >/dev/null && { sleep 1; w; }; }; w' \
  $'# wait for the job, it\'s slow\nuntil ! pgrep -f qqzr47 >/dev/null; do sleep 1; done' \
  $'sleep 1  # let\'s give it a moment\nwhile pgrep -f qqzr51 >/dev/null; do sleep 1; done' \
  $'# poll every 5" or so\nuntil ! pgrep -f qqzr55 >/dev/null; do sleep 1; done' \
  $'cat > /tmp/qqn.txt <<\'X\'\nthe job\'s still running\nX\nuntil ! pgrep -f qqzr50 >/dev/null; do sleep 1; done' \
  $'# block until the job\'s gone\npidwait -f qqzr71' 'waitpid $(pgrep -f qqzr44)' 'tail --pi=$(pgrep -of qqzr45) -f /dev/null' \
  'tail -f /dev/null --pi $(pgrep -of qqzr67)' 'until ! pg\rep -f qqzr46 >/dev/null; do sleep 1; done' \
  "until ! pgr''ep -f qqzr46 >/dev/null; do sleep 1; done" 'pid\wait -f qqzr46' \
  'until [ "`echo 0`:`pgrep -cf qqzr65`" = 0:0 ]; do sleep 1; done' \
  "bash -c 'until ! pgrep -x bash >/dev/null; do sleep 1; done'" $'bash -c $\'until ! pgrep -f qqzr6\\x36; do sleep 1; done\'' \
  "until ! pgrep -f 'tmp[}]{3}/claude' >/dev/null; do sleep 1; done" \
  "until ! pgrep -f ~/'[.]claude/shell-snapshots' >/dev/null; do sleep 1; done"; do
  check DENY "$me" "$line"
done
# refused though they end: the hook doesn't tell which pgrep on the line a wait rests on, so a listing one goes too
for line in 'while sleep 5; do date; done; pgrep -f foo' 'wait-gone $(cat job.pid); pgrep -f foo' \
  'git commit -m "until ! pgrep -f foo; do sleep 5; done"' \
  'python3 qqjob7.py & pid=$!; sleep 1; pgrep -af qqjob7; ~/workspace/claude/tools/wait-gone --timeout 20 $pid' \
  'while [ ! -e out/result.json ]; do pgrep -af train.py | tail -2; sleep 30; done' \
  'sleep 3 & pid=$!; while :; do kill -0 $pid 2>/dev/null || break; pgrep -af train.py | tail -1; sleep 1; done' \
  'while pgrep -af qqjob7 | command grep -v pgrep >/dev/null; do sleep 1; done' 'while read -r p; do pgrep -af "$p"; done < /dev/null' \
  'for p in a b; do pgrep -f $p; done'; do
  check DENY "$me" "$line"
done
for line in "pgrep -f '[r]ows27'" "until ! pgrep -f '[x]yz'; do sleep 5; done" "wait-gone \$(pgrep -f '[c]ompare-finish-job19')" \
  'pgrep -f foo' 'until ! pgrep foo; do sleep 5; done' \
  "for i in \$(seq 600); do pgrep -f '[q]qzr40' >/dev/null || break; sleep 1; done; echo ended-\$i" \
  'if pgrep -f qqjob7 >/dev/null; then echo up; fi' 'pgrep -f qqjob7 >/dev/null && echo up' \
  "until ! pgrep -f '[Qq]qjob7'; do sleep 1; done" "until ! pgrep -f 'qqjob[7]'; do sleep 1; done" \
  "until ! pgrep -u ironmoon -f '[q]qcargo test' >/dev/null; do sleep 1; done" \
  "until ! pgrep -u \"\$USER\" -f '[q]qcargo test' >/dev/null; do sleep 1; done" \
  "until ! pgrep -f -u \$USER '[q]qcargo test' >/dev/null; do sleep 1; done" "until ! pgrep -O 5 -f '[q]qcargo test'; do sleep 1; done" \
  "pgrep -af '[q]qcargo test'; until ! pgrep -f '[q]qcargo test' >/dev/null; do sleep 1; done" \
  "until ! pgrep -f '[q]qcargo test' >/dev/null; do sleep 1; done; cat pgrep.log" \
  $'# wait for the run (pgrep)\nuntil ! pgrep -f \'[q]qcargo test\' >/dev/null; do sleep 1; done' \
  "until ! pgrep -f 'train\\.py\$' >/dev/null; do sleep 1; done" "until ! pgrep -f '(a' >/dev/null; do sleep 1; done" \
  "until ! pgrep -f \"[t]rain\\.py --resume\" >/dev/null; do sleep 1; done" "until ! pgrep -f \"qqz's job\" >/dev/null; do sleep 1; done" \
  "while pgrep -fx 'python3 qqjob7.py'; do sleep 1; done" "until [ -z \"\`pgrep -f '[q]qjob7'\`\" ]; do sleep 1; done" \
  "bash -c 'until ! pgrep -f '\\''[q]q job'\\''; do :; done'"; do
  check PASS "$me" "$line"
done
says "$me" 'until pgrep -f qqjob7 >/dev/null; do sleep 1; done' 'waiting until it finds something ends at once'
says "$me" 'until ! pgrep -f "$pat" >/dev/null; do sleep 1; done' "can't read this \`pgrep\`'s pattern"
says "$me" 'pidwait -f qqjob7' 'waits for that shell too and never returns'

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
