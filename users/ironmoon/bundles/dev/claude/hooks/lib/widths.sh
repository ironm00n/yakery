# The commit width rule a push from a Claude Code session is held to: each commit it sends that isn't on the remote
# yet keeps its first line and body within $width characters (lib/commit-width.jq). One width fits each of Owen's
# commit rules: yakery's 76, redox's 72 and spytial's lazygit wrap. jj-push.sh checks jj pushes and
# git-pre-push.sh plain git ones.

width=72
. "${BASH_SOURCE[0]%/*}/pushed.sh"

# Of the commits a push sends (pushed.sh, its pre-push lines on stdin), one `<sha> <problem>` line each for those whose
# message breaks the rule, <problem> being commit-width.jq's object.
too_wide() {
  pushed_log -z --format='%H%n%B' | jq -Rsr --argjson width "$width" -f "${BASH_SOURCE[0]%/*}/commit-width.jq"
}

# Why a push is refused, from too_wide's lines in $1, each commit named by command $2 given its sha, and how to fix it,
# by $3.
too_wide_reason() {
  local sha problem name what
  echo "these commits break the commit width rule, a first line and body lines within $width characters (a line that can't break, a fenced block or the closing trailers excepted):"
  while read -r sha problem; do
    name=$($2 "$sha")
    what=$(jq -r --argjson width "$width" '[
      (.subject | select(. != null) | "its first line is \(.) characters"),
      (.lines | select(length > 0) | "body line\(if length > 1 then "s \(map(tostring) | join(", ")) run" else " \(.[0]) runs" end) over")
    ] | join(", and ")' <<<"$problem")
    echo "  $name: $what"
  done <<<"$1"
  echo "$3 A first line is \`topic: what changed\`, as short as Owen's own (about 32 characters); a why that isn't obvious goes in one or two body lines, and a short body now and then is fine."
}
