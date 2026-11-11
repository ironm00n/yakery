#!/usr/bin/env bash
# usage: items.sh NAME REPO [OWN]
# The open issues of the forge's repository REPO given to the lane NAME by an owner/NAME or owner:NAME label, most
# recently updated first, each with the newest comment not among the ids in the JSON array OWN (the lane's own) and
# the newest that is, as one JSON array. The tracker answers a label filter naming a label it lacks with every
# issue, so only labels it has are asked for, and each issue's labels are checked again here.
set -euo pipefail
shopt -s inherit_errexit
name=${1:?usage: items.sh NAME REPO [OWN]} repo=${2:?usage: items.sh NAME REPO [OWN]} own=${3:-[]}
source "$(dirname "$(realpath "$0")")/tracker.sh"
owned='.name == "owner/\($n)" or .name == "owner:\($n)"'
issues=$(all 'labels?' | jq -r --arg n "$name" ".[] | select($owned) | .name | @uri" | while read -r label; do
	all "issues?state=open&type=issues&labels=$label"
done | jq -s --arg n "$name" "add // [] | map(select(any(.labels[]; $owned))) | unique_by(.number) | sort_by(.updated_at) | reverse")
# held until every request is in, so a failure prints nothing rather than a list that looks whole
listed=$(jq -c '.[] | select(.comments > 0) | .number' <<<"$issues" | while read -r number; do
	get "issues/$number/comments" | jq --argjson n "$number" '{($n | tostring): .}'
done | jq -s --argjson own "$own" --slurpfile issues <(printf '%s\n' "$issues") '
	def newest(f): [.[] | select(.id | IN($own[]) | f)] | last | if . then {by: .user.login, at: .created_at, body} else null end;
	add // {} as $comments | $issues[0] | map(($comments[.number | tostring] // []) as $all | {
		number, title, url: .html_url, updated: .updated_at, labels: [.labels[].name], body,
		newest: ($all | newest(not)), mine: ($all | newest(.))
	})')
printf '%s\n' "$listed"
