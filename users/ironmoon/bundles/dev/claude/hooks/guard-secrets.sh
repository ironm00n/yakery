#!/usr/bin/env bash
# PreToolUse guard for what never enters a transcript: ~/.claude/secrets, whose keys are used only
# inline as `$(<path)`; sops-nix's decrypted /run/secrets, which a command may list but not read; and
# the private keys in ~/.ssh, which only ssh's own tools may take. Denies Read/Edit/Write/Grep/Glob of
# any of them, and Bash that reads one any other way. Catches slips, not intent: a path built at
# runtime passes.

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

secrets=$(realpath -m "$HOME/.claude/secrets")
keys=$(realpath -m "$HOME/.ssh")

deny() {
  jq -cn --arg r "Blocked: $1." '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}
deny_secrets() {
  deny "$1. Keys in ~/.claude/secrets are used only inline, as \`\$(<~/.claude/secrets/<name>)\` inside the command that needs them, and never printed"
}

# Which sealed place absolute path $1 lies in, if any.
sealed() {
  case $1 in
    "$secrets" | "$secrets"/*) echo secrets ;;
    /run/secrets | /run/secrets/* | /run/secrets.d | /run/secrets.d/*) echo sops ;;
    "$keys"/*.pub | "$keys"/known_hosts* | "$keys"/authorized_keys* | "$keys"/config) ;;
    "$keys" | "$keys"/*) echo keys ;;
  esac
}

# The programs that may name a place's files on a command line without printing or copying them.
takes() {
  case $1:$2 in
    sops:ls | sops:stat) ;;
    keys:ssh | keys:ssh-keygen | keys:ssh-add | keys:ls | keys:stat | keys:chmod) ;;
    *) return 1 ;;
  esac
}

tool=$(jq -r '.tool_name // ""' <<<"$payload")

if [[ $tool == Bash ]]; then
  cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
  [[ $cmd == *.claude/secrets* || $cmd == */run/secrets* || $cmd == *.ssh* ]] || exit 0
  cwd=$(dir_of "$(jq -r '.cwd // ""' <<<"$payload")" "")
  if [[ $cmd == *.claude/secrets* ]]; then
    inline='\$\(<[[:space:]]*(~|\$HOME|\$\{HOME\}|/home/[^/]+)/\.claude/secrets/[[:alnum:]_.-]+\)'
    grep -Eq "(^|[;&|(])[[:space:]]*(echo|printf)[^;&|]*${inline}" <<<"$cmd" &&
      deny_secrets "\`echo\`/\`printf\` would print the key"
    [[ $(sed -E "s#${inline}##g" <<<"$cmd") == *.claude/secrets* ]] &&
      deny_secrets "this names ~/.claude/secrets outside an inline \`\$(<...)\`"
  fi
  while read -r target; do
    p=$(dir_of "$target" "$cwd")
    place=$(sealed "$p")
    [[ -n $place && $place != secrets ]] && deny "a redirection would read or write $p"
  done < <(grep -oE '[0-9]*[<>]+&?[[:space:]]*[^[:space:];&|()<>]+' <<<"$cmd" | sed -E 's/^[0-9]*[<>]+&?[[:space:]]*//')
  check() {
    local x p place
    for x in "${w[@]:1}"; do
      for p in "$x" "${x#*=}"; do
        p=$(dir_of "$p" "$here")
        place=$(sealed "$p")
        [[ -n $place && $place != secrets ]] && ! takes "$place" "${w[0]##*/}" &&
          deny "\`${w[0]##*/}\` would read $p; only $([[ $place == sops ]] && echo 'ls and stat name /run/secrets' || echo "ssh's own tools, ls, stat and chmod name a private key")"
      done
    done
  }
  each_command check "$cmd" "$cwd" ''
  exit 0
fi

cwd=$(jq -r '.cwd // empty' <<<"$payload")
while IFS= read -r p; do
  [[ $p == \~ || $p == \~/* ]] && p=$HOME${p:1}
  [[ $p == /* ]] || p=${cwd:-$PWD}/$p
  p=$(realpath -m "$p")
  case $(sealed "$p") in
    secrets) deny_secrets "$tool would open $p" ;;
    sops | keys) deny "$tool would open $p" ;;
  esac
done < <(jq -r 'if .tool_name == "Grep" then .tool_input.path
                else .tool_input | (.file_path, .notebook_path, .path, .pattern) end | values' <<<"$payload")

exit 0
