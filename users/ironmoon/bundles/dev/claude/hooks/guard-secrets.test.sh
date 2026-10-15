h="${1:-$HOME/.claude/hooks/guard-secrets.sh}"
check() {
  out=$(jq -cn --arg t "$2" --argjson i "$3" '{tool_name:$t, tool_input:$i, cwd:env.HOME}' | bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %s %s\n' "$got" "$2" "$3" || printf 'FAIL want=%s got=%s  %s %s\n' "$1" "$got" "$2" "$3"
}
cmd() { check "$1" Bash "$(jq -cn --arg c "$2" '{command:$c}')"; }

cmd DENY 'cat ~/.claude/secrets/virustotal'
cmd DENY 'cat $HOME/.claude/secrets/virustotal'
cmd DENY 'ls -la ~/.claude/secrets'
cmd DENY 'cp ~/.claude/secrets/virustotal /tmp/k'
cmd DENY 'echo "$(<~/.claude/secrets/virustotal)"'
cmd DENY 'printf %s "$(< ~/.claude/secrets/virustotal)" | wc -c'

cmd PASS 'curl -s -H "x-apikey: $(<~/.claude/secrets/virustotal)" https://www.virustotal.com/api/v3/files/abc'
cmd PASS 'curl -s -H "Auth-Key: $(<$HOME/.claude/secrets/abuse-ch)" -d query=get_info https://mb-api.abuse.ch/api/v1/'
cmd PASS 'rg -n secrets /etc/nixos'
cmd PASS 'jq . ~/.claude/settings.json'

check DENY Read '{"file_path":"~/.claude/secrets/virustotal"}'
check DENY Edit '{"file_path":".claude/secrets/virustotal"}'
check DENY Grep '{"pattern":"x","path":"~/.claude/secrets"}'
check DENY Glob '{"pattern":"~/.claude/secrets/*"}'

check PASS Read '{"file_path":"~/.claude/settings.json"}'
check PASS Grep '{"pattern":".claude/secrets","path":"/etc/nixos"}'
check PASS Glob '{"pattern":"**/*.nix"}'
