#!/usr/bin/env bash
# usage: items.sh NAME
# The open agents/work issues given to the lane NAME by an owner/NAME or owner:NAME label, most recently updated first,
# each with its newest comment, as one JSON array. The tracker answers a label filter naming a label it lacks with every
# issue, so only labels it has are asked for, and each issue's labels are checked again here.
set -euo pipefail
shopt -s inherit_errexit
name=${1:?usage: items.sh NAME}
api=${LANE_TRACKER:-https://code.ironmoon.dev/api/v1/repos/agents/work}
# the token reaches curl as config on stdin, so no command line holds it
get() { curl -fsS --noproxy '*' --max-time 20 -K - "$api/$1" <<<"header = \"Authorization: token $(<~/.claude/secrets/forgejo-agent-pat)\""; }
# every page of a listing, as one array; 20 pages of 50 is past anything the tracker holds
all() {
	local page items
	for page in {1..20}; do
		items=$(get "$1&limit=50&page=$page")
		[[ $items != '[]' ]] || break
		printf '%s\n' "$items"
	done | jq -s 'add // []'
}
owned='.name == "owner/\($n)" or .name == "owner:\($n)"'
issues=$(all 'labels?' | jq -r --arg n "$name" ".[] | select($owned) | .name | @uri" | while read -r label; do
	all "issues?state=open&type=issues&labels=$label"
done | jq -s --arg n "$name" "add // [] | map(select(any(.labels[]; $owned))) | unique_by(.number) | sort_by(.updated_at) | reverse")
# held until every request is in, so a failure prints nothing rather than a list that looks whole
listed=$(jq -c '.[] | select(.comments > 0) | .number' <<<"$issues" | while read -r number; do
	get "issues/$number/comments" | jq --argjson n "$number" '{($n | tostring): last}'
done | jq -s --slurpfile issues <(printf '%s\n' "$issues") 'add // {} as $newest | $issues[0] | map({
	number, title, url: .html_url, updated: .updated_at, labels: [.labels[].name], body,
	newest: ($newest[.number | tostring] | if . then {by: .user.login, at: .created_at, body} else null end)
})')
printf '%s\n' "$listed"
