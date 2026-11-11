# Sourced by items.sh and mine.sh: requests to the forge's repository $repo (or $LANE_TRACKER), as the lane's agent token.
api=${LANE_TRACKER:-https://code.ironmoon.dev/api/v1/repos/$repo}
# the token reaches curl as config on stdin, so no command line holds it
get() { curl -fsS --noproxy '*' --max-time 20 -K - "$api/$1" <<<"header = \"Authorization: token $(<~/.claude/secrets/forgejo-agent-pat)\""; }
# every page of a listing, as one array: a short page is the last, and 200 pages of 50 is past anything it holds
all() {
	local page items
	for page in {1..200}; do
		items=$(get "$1&limit=50&page=$page")
		printf '%s\n' "$items"
		(($(jq length <<<"$items") == 50)) || break
	done | jq -s 'add // []'
}
