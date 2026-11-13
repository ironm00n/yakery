# The provenance rule a push from a Claude Code session is held to: each commit it sends that isn't on a remote yet and
# is authored as the agent identity carries a Generated-by trailer and is signed by the agent's key. jj-push.sh checks jj
# pushes and git-pre-push.sh plain git ones.

. "${BASH_SOURCE[0]%/*}/pushed.sh"

# The agent identity as the claude bundle gives a session's commits: its address, the gpg that reads its keyring, and
# its key, the one it names or, as git and jj take it, its address's.
identity() { git config --file "${XDG_CONFIG_HOME:-$HOME/.config}/git/claude-identity.user" "$1"; }
agent_email() { identity user.email; }
agent_gpg() { identity gpg.program || echo gpg; }
agent_key() {
  "$(agent_gpg)" --with-colons --list-keys -- "$(identity user.signingkey || agent_email)" 2>/dev/null |
    awk -F: '$1 == "fpr" { print $10; exit }'
}

# Of the commits a push sends, the sha of each authored as the agent. Fails when the agent's address can't be read.
agent_commits() {
  local agent
  agent=$(agent_email) && [[ -n $agent ]] || return 1
  pushed_log --format='%H %ae' | awk -v agent="${agent,,}" 'tolower($2) == agent { print $1 }'
}

# Of those, each not signed good by the agent's key, as `<sha> <%G?> <signing key's fingerprint>`, verified through the
# agent's gpg; with no such key in its keyring, every one. Fails when the agent's address can't be read.
unsigned() (
  agent=$(agent_email) && [[ -n $agent ]] || exit 1
  gpg=$(agent_gpg) key=$(agent_key)
  git() { command git -c gpg.program="$gpg" "$@"; }
  pushed_log --format='%H %ae %G? %GP' | awk -v agent="${agent,,}" -v key="$key" \
    'tolower($2) == agent && ($3 != "G" || key == "" || $4 != key) { print $1, $3, $4 }'
)

# Why a push is refused, from unsigned's lines in $1, each commit named by command $2 given its sha, and how to fix it,
# by $3.
unsigned_reason() {
  local sha status key own
  own=$(agent_key)
  echo "these commits are authored as $(agent_email) and aren't signed good by its key ${own:-(none in the keyring $(agent_gpg) reads)}:"
  while read -r sha status key; do echo "  $($2 "$sha") ($status${key:+, by $key})"; done <<<"$1"
  echo "$3"
}

# Of the commits a push sends (pushed.sh, its pre-push lines on stdin), the sha of each authored as the agent with no
# Generated-by trailer. Fails when the agent's address can't be read.
unmarked() {
  local agent
  agent=$(agent_email) && [[ -n $agent ]] || return 1
  pushed_log --format='%H %ae %(trailers:key=Generated-by,valueonly,separator=%x2C)' |
    awk -v agent="${agent,,}" 'tolower($2) == agent && NF == 2 { print $1 }'
}

# Why a push is refused, from unmarked's shas in $1, each commit named by command $2 given its sha, and how to fix it,
# by $3.
unmarked_reason() {
  local sha
  echo "these commits are authored as $(agent_email) and carry no Generated-by trailer, which every commit of the agent identity ends with:"
  while read -r sha; do echo "  $($2 "$sha")"; done <<<"$1"
  echo "$3 The trailer is the session's provenance line, \`Generated-by: <model>:<effort> claude-code:<version>\`; an Assisted-by line alone doesn't do."
}
