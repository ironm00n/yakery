#!/usr/bin/env bash
# usage: workspace.sh jj|git ROOT <IDS
# One workspace as commits.sh lists it: its working copy, the newest commits below it that trunk lacks with how many
# there are, and the commits among IDS (one a line) that neither of those holds, as one JSON object. Of IDS only commits
# still there count: visible ones in jj, which finds an abandoned commit by its id too, and ones a ref leads to in git;
# and no empty one without a description, a working copy's. Only reads: jj never snapshots the working copy, and git
# takes no optional locks.
set -euo pipefail
shopt -s inherit_errexit
vcs=${1:?usage: workspace.sh jj|git ROOT <IDS} root=${2:?usage: workspace.sh jj|git ROOT <IDS}
SHOWN=10
mapfile -t ids

jjq() { jj --ignore-working-copy --no-pager --color never -R "$root" "$@"; }
JJ='"{\"change\":" ++ json(change_id.short(8)) ++ ",\"commit\":" ++ json(commit_id.short(8))
	++ ",\"at\":" ++ json(committer.timestamp().utc().format("%Y-%m-%dT%H:%M:%SZ")) ++ ",\"subject\":" ++ json(description.first_line())
	++ ",\"bookmarks\":" ++ json(local_bookmarks.map(|b| b.name())) ++ ",\"pushed\":" ++ json(self.contained_in("::remote_bookmarks()"))
	++ ",\"empty\":" ++ json(empty) ++ "}\n"'
jj_log() { jjq log --no-graph -T "$JJ" "$@" | jq -s .; }
jj_named() {
	((${#ids[@]} > 0)) || return 0
	# one prefix that two commits share fails the query of them all, so then each is asked alone
	local others='~ (trunk()..@) ~ (empty() & description(exact:""))' id
	jj_log -r "(($(printf 'commit_id(%s) | ' "${ids[@]}")none()) & ::visible_heads()) $others" -n $SHOWN 2>/dev/null && return
	for id in "${ids[@]}"; do jj_log -r "(commit_id($id) & ::visible_heads()) $others" 2>/dev/null || true; done
}
list_jj() {
	local head line count named
	head=$(jj_log -r @ | jq '.[0]')
	line=$(jj_log -r 'trunk()..@-' -n $SHOWN)
	count=$(jjq log --no-graph -r 'trunk()..@-' -T '"\n"' | wc -l)
	named=$(jj_named | jq -s --argjson shown $SHOWN 'add // [] | unique_by(.commit) | sort_by(.at) | reverse | .[:$shown]')
	jq -n --arg root "$root" --argjson head "$head" --argjson line "$line" --argjson count "$count" --argjson named "$named" \
		'{$root, vcs: "jj", $head, $line, more: ($count - ($line | length)), $named}'
}

gitq() { git --no-optional-locks -C "$root" "$@"; }
git_log() {
	TZ=UTC0 gitq log --decorate-refs=refs/heads --date=format-local:%Y-%m-%dT%H:%M:%SZ --format='%h%x1f%cd%x1f%s%x1f%D' "$@" |
		while IFS=$'\x1f' read -r commit at subject refs; do
			jq -cn --arg commit "$commit" --arg at "$at" --arg subject "$subject" --arg refs "$refs" \
				--argjson pushed "$([[ -n $(gitq for-each-ref --contains "$commit" --count=1 --format=x refs/remotes) ]] && echo true || echo false)" \
				'{$commit, $at, $subject, bookmarks: ($refs | split(", ") | map(sub("^HEAD -> "; "")) | map(select(. != "" and . != "HEAD"))), $pushed, empty: false}'
		done | jq -s .
}
# The branch's upstream, else the remote's HEAD, else the first of the remote branches jj's trunk() looks for.
git_trunk() {
	local ref
	for ref in '@{upstream}' origin/HEAD {origin,upstream}/{main,master,trunk}; do
		gitq rev-parse --verify -q --symbolic-full-name "$ref" && return
	done
}
list_git() {
	local trunk below head line count changed named=()
	trunk=$(git_trunk || true)
	below=(${trunk:+"^$trunk"} 'HEAD^@')
	head=$(git_log -n1 HEAD | jq '.[0]')
	line=$(git_log -n $SHOWN "${below[@]}")
	count=$(gitq rev-list --count "${below[@]}")
	changed=$(gitq status --porcelain | wc -l)
	if ((${#ids[@]} > 0)); then
		mapfile -t named < <(printf '%s\n' "${ids[@]}" | gitq cat-file --batch-check='%(objecttype) %(objectname)' | sed -n 's/^commit //p' |
			grep -vxFf <(gitq rev-list ${trunk:+"^$trunk"} HEAD) | while read -r id; do
				[[ -z $(gitq for-each-ref --contains "$id" --count=1 --format=x) ]] || echo "$id"
			done || true)
	fi
	jq -n --arg root "$root" --argjson head "$head" --argjson line "$line" --argjson count "$count" --argjson changed "$changed" \
		--argjson named "$(if ((${#named[@]} > 0)); then git_log --no-walk=sorted -n $SHOWN "${named[@]}"; else echo '[]'; fi)" \
		'{$root, vcs: "git", $head, $changed, $line, more: ($count - ($line | length)), $named}'
}

case $vcs in jj | git) "list_$vcs" ;; *) echo "$vcs: neither jj nor git" >&2 && exit 2 ;; esac
