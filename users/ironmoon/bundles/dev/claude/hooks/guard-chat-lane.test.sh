h="${1:-$HOME/.claude/hooks/guard-chat-lane.sh}"
check() {
  out=$(jq -cn --arg to "$2" '{tool_name:"SendMessage", tool_input:{to:$to, message:"hi"}}' | bash "$h")
  got=$(jq -r '.hookSpecificOutput.permissionDecision // "pass"' <<<"${out:-{\}}")
  [ "$got" = "$1" ] && printf 'ok   %-5s %s\n' "$1" "$2" || printf 'FAIL want=%s got=%s to=%s\n' "$1" "$got" "$2"
}

check deny 'chat'
check deny 'chat [32b6c5]'
check deny '  Chat  '
check pass 'dispatch'
check pass 'chatter'
check pass 'chat-2'
check pass 'uds:/run/user/1000/cc-socks/2463313.sock'
