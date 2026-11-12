# The provenance rule a push from a Claude Code session is held to: each commit it sends that isn't on a remote yet and
# is authored as the agent identity carries a Generated-by trailer. jj-push.sh checks jj pushes and git-pre-push.sh plain git ones.

. "${BASH_SOURCE[0]%/*}/pushed.sh"

# The agent identity's address, as the claude bundle gives a session's commits.
agent_email() { git config --file "${XDG_CONFIG_HOME:-$HOME/.config}/git/claude-identity.user" user.email; }

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
