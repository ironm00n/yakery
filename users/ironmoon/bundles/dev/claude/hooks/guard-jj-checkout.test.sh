h="${1:-$HOME/.claude/hooks/guard-jj-checkout.sh}"
dir=$(realpath "$(mktemp -d)")
trap 'rm -r "${dir:?}"' EXIT
for d in workspace/redox/.jj workspace/redox/sub workspace/redox/nested/.jj workspace/claude/.jj \
  research/r/.jj research/r/src/deep research/plain tmp/worktree/x/.jj tmp/worktree/x/sub elsewhere; do
  mkdir -p "$dir/$d"
done
projects="$dir/.claude/projects/${dir//[^[:alnum:]]/-}"
dispatch="{\"transcript_path\":\"$projects-workspace-claude/s1.jsonl\"}"

# check WANT CWD COMMAND [EXTRA-INPUT-JSON] [CLAUDE_JOB_DIR]
check() {
  extra=${4:-'{}'}
  out=$(jq -cn --arg c "$3" --arg cwd "$2" --argjson x "$extra" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd} + $x' |
    env -u CLAUDE_JOB_DIR ${5:+CLAUDE_JOB_DIR=$5} HOME="$dir" bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %-24s %s\n' "$got" "${2#"$dir"}" "${3//"$dir"/\~}" ||
    printf 'FAIL want=%s got=%s %s  %s\n' "$1" "$got" "${2#"$dir"}" "${3//"$dir"/\~}"
}
r="$dir/workspace/redox"
c="$dir/workspace/claude"
e="$dir/elsewhere"

check DENY "$r" 'jj st'
check DENY "$r/sub" 'jj --no-pager log -r @ -n1'
check DENY "$c" 'jj st'
check DENY "$dir/research/r/src/deep" 'jj diff'
check DENY "$e" 'jj -R ~/workspace/redox log'
check DENY "$e" 'jj log -R "$HOME/workspace/redox/sub"'
check DENY "$e" 'jj --repository=~/workspace/redox log'
check DENY "$e" "jj -R$r log"
check DENY "$e" 'jj -R /etc/nixos log'
check DENY "$e" 'cd ~/workspace/redox && jj log'
check DENY "$e" 'cd ../workspace/redox; jj log'
check DENY "$e" 'if cd ~/workspace/redox; then jj log; fi'
check DENY "$e" 'env -C ~/workspace/redox jj log'
check DENY "$r" 'echo "$(jj log -r @)"'
check DENY "$r" 'x=`jj root`'
check DENY "$r" 'echo "at $(cd /tmp; ls) and `jj root`"'
check DENY "$r" 'jj \
  st'
check DENY "$r" 'true; jj st'
check DENY "$r" 'git status && jj diff | head'
check DENY "$r" 'FOO=1 command timeout 10 /run/current-system/sw/bin/jj st'
check DENY "$r" 'echo abc | xargs -n1 jj abandon'
check DENY "$r" '(cd ~/tmp/worktree/x && jj st); jj st'
check DENY "$e" 'cd "$ws" && jj st'
check DENY "$e" 'jj -R "$ws" log'
check DENY "$e" 'jj -R $(pwd) log'
check DENY "$e" 'cd ~/workspace/redox && cd - && jj st'
check DENY "$r" 'jj st -- --ignore-working-copy'

check PASS "$r" 'jj --ignore-working-copy st'
check PASS "$e" 'jj -R ~/workspace/redox --ignore-working-copy log'
check PASS "$e" 'cd ~/workspace/redox && jj file show -r @ README.md --ignore-working-copy'
check PASS "$r" 'jj -R ~/tmp/worktree/x log'
check PASS "$r" 'jj -R ../../tmp/worktree/x/sub log'
check PASS "$r" 'cd ~/tmp/worktree/x && jj st'
check PASS "$e" '(cd ~/workspace/redox && ls); jj st'
check PASS "$r/nested" 'jj st'
check PASS "$dir/research/plain" 'jj st'
check PASS "$e" 'jj st'
check PASS "$r" "rg 'jj st' notes"
check PASS "$r" 'git commit -m "fix; then jj st -R ~/workspace/redox"'
check PASS "$r" 'echo jj st'
check PASS "$r" 'ls  # then jj st'
check PASS "$r" 'git commit -m "first line
then; jj st -R ~/workspace/redox"'
check PASS "$r" 'ls -la && git log -1'
check PASS "$r" 'cat <<EOF > notes.md
jj st
EOF'

check PASS "$c" 'jj rebase -d master' "$dispatch"
check PASS "$e" 'jj -R ~/workspace/claude git fetch' "$dispatch"
check DENY "$c" 'jj rebase -d master' '{"agent_id":"a1","transcript_path":"'"$projects"'-workspace-claude/s1.jsonl"}'
check DENY "$c" 'jj rebase -d master' "$dispatch" "$dir/jobs/x"
check DENY "$c" 'jj rebase -d master' "{\"transcript_path\":\"$projects-workspace-redox/s1.jsonl\"}"
check DENY "$c" 'cd ~/workspace/redox && jj st' "$dispatch"
