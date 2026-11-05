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

cmd DENY 'cat /run/secrets/explorer-ssh-data'
cmd DENY 'cat /run/secrets.d/1/gpg-signing_key'
cmd DENY 'cp /run/secrets/explorer-ssh-data /tmp/k'
cmd DENY 'base64 < /run/secrets/explorer-ssh-data'
cmd DENY 'k=$(cat /run/secrets/explorer-ssh-data); echo ok'
cmd DENY 'sudo head -c 64 /run/secrets/explorer-ssh-data'
cmd DENY 'cat ~/.ssh/id_ed25519'
cmd DENY 'head -c 100 $HOME/.ssh/duckham.o-20261027T134519Z'
cmd DENY 'cp ~/.ssh/id_ed25519 /tmp/'
cmd DENY 'tar -cf /tmp/ssh.tar ~/.ssh'
cmd DENY 'scp ~/.ssh/id_ed25519 host:'
cmd DENY 'cd ~/.ssh && cat id_ed25519'
cmd DENY 'xxd < ~/.ssh/id_ed25519'

cmd PASS 'ls -l /run/secrets/'
cmd PASS 'stat -c %U:%a /run/secrets/explorer-ssh-data'
cmd PASS 'ssh-keygen -lf ~/.ssh/id_ed25519'
cmd PASS 'ssh -o BatchMode=yes -i ~/.ssh/duckham.o-20261027T134519Z duckham.o@login.explorer.northeastern.edu true'
cmd PASS 'chmod 600 ~/.ssh/id_ed25519'
cmd PASS 'ls -la ~/.ssh'
cmd PASS 'cat ~/.ssh/id_ed25519.pub'
cmd PASS 'cat ~/.ssh/known_hosts | wc -l'
cmd PASS 'ssh-keygen -lf ~/.ssh/authorized_keys'
cmd PASS 'rg -n run/secrets /etc/nixos'

check DENY Read '{"file_path":"/run/secrets/explorer-ssh-data"}'
check DENY Read '{"file_path":"~/.ssh/id_ed25519"}'
check DENY Grep '{"pattern":"PRIVATE","path":"~/.ssh"}'
check DENY Glob '{"pattern":"/run/secrets/*"}'
check DENY Edit '{"file_path":"~/.ssh/duckham.o-20261027T134519Z"}'

check PASS Read '{"file_path":"~/.ssh/config"}'
check PASS Read '{"file_path":"~/.ssh/id_ed25519.pub"}'
check PASS Read '{"file_path":"~/.ssh/known_hosts"}'
