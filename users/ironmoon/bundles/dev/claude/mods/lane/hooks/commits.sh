#!/usr/bin/env bash
# usage: commits.sh CWD [TRANSCRIPT]
# The jj and git workspaces a session used, those worked.jq finds in TRANSCRIPT in its order and then CWD, each as
# workspace.sh lists it given the commit ids worked.jq found, as one JSON array. A named commit is left out where any
# workspace's working copy or line holds it, and named once at most. A workspace that can't be listed is there with the
# last line its listing printed as `error`.
set -euo pipefail
shopt -s inherit_errexit
cwd=${1:?usage: commits.sh CWD [TRANSCRIPT]} transcript=${2:-}
here=$(dirname "$0") WORKSPACES=6
worked='{"dirs": [], "ids": []}'
[[ -z $transcript || ! -s $transcript ]] || worked=$(jq -cnR --arg home "$HOME" -f "$here/worked.jq" "$transcript")
err=$(mktemp) && trap 'rm -f "$err"' EXIT
declare -A seen=()
found=0
while IFS= read -r dir; do
	dir=$(realpath -eq -- "$dir") && [[ -d $dir ]] || continue
	# from the directory itself, since -R names a workspace's root and finds none from below it
	if root=$(cd -- "$dir" && jj --ignore-working-copy workspace root 2>/dev/null); then vcs=jj
	elif root=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null); then vcs=git
	else continue; fi
	[[ -z ${seen[$root]:-} ]] || continue
	seen[$root]=1
	jq -r '.ids[]' <<<"$worked" | bash "$here/workspace.sh" "$vcs" "$root" 2>"$err" ||
		jq -n --arg root "$root" --arg vcs "$vcs" --arg error "$(grep . "$err" | tail -1)" '{$root, $vcs, $error}'
	((++found < WORKSPACES)) || break
done < <(jq -r --arg cwd "$cwd" '.dirs[], $cwd' <<<"$worked") | jq -s '
	def ids: map({(.commit): true}) | add // {};
	reduce .[] as $w ({seen: (map(.head // empty, .line[]?) | ids), out: []};
		.seen as $seen | ($w | if .error then . else .named |= map(select($seen[.commit] | not)) end) as $w
		| .out += [$w] | .seen += ($w.named // [] | ids))
	| .out'
