#!/usr/bin/env bash
# usage: mine.sh TRANSCRIPT REPO
# The comments and issues on the forge's repository REPO that the session of TRANSCRIPT posted, as {comments: [ids],
# issues: [numbers]}. All lanes post as the one agent account, so the tracker can't say whose they are; the session's
# transcript can. One made while it ran (from its first row to its last) is its own when its whole text, in letters
# and digits only, is in what the session sent for it (posting.jq: the command that posted the comment to that issue,
# or edited it, or filed the issue, and the files the command sent), or when such a command printed its id or number
# (posted.jq), one of the tracker's. A quote of it anywhere else claims nothing.
set -euo pipefail
shopt -s inherit_errexit
transcript=${1:?usage: mine.sh TRANSCRIPT REPO} repo=${2:?usage: mine.sh TRANSCRIPT REPO}
here=$(dirname "$(realpath "$0")")
source "$here/tracker.sh"
work=$(mktemp -d) && trap 'rm -rf "$work"' EXIT
# A command spells a newline in JSON as \n, which the comment holds as a newline; both, as JSON, leave only letters and
# digits. Each tool input is one line, so no text is matched across two.
plain() { sed -E 's/\\\\[nrt]|\\[nrt]/ /g' | tr 'A-Z' 'a-z' | tr -cd 'a-z0-9\n'; }
since=$(jq -rnR 'first(inputs | fromjson? | .timestamp? // empty) | @uri' "$transcript")
before=$(tail -n 100 "$transcript" | jq -rnR '[inputs | fromjson? | .timestamp? // empty] | last // empty | @uri')
all "issues/comments?since=$since&before=$before" >"$work/comments"
all "issues?state=all&type=issues&since=$since&before=$before" >"$work/filed"
jq -cnR -f "$here/posted.jq" "$transcript" >"$work/printed" &
printing=$!
jq -rnR -f "$here/posting.jq" "$transcript" >"$work/posting"
cut -f1 "$work/posting" >"$work/where"
cut -f2- "$work/posting" | plain >"$work/written"
# each comment's and issue's text, as plain, beside what it is and where its text was sent; a text this short could
# be anyone's. Each jq runs on its own, so a listing it can't read fails the run rather than finding nothing.
jq -r '.[] | "comments\t\(.id)\t\(.issue_url | split("/") | last)"' "$work/comments" >"$work/heads"
jq -r '.[] | "issues\t\(.number)\tnew"' "$work/filed" >>"$work/heads"
jq -r '.[].body | tojson' "$work/comments" >"$work/texts"
jq -r '.[].body // "" | tojson' "$work/filed" >>"$work/texts"
paste "$work/heads" <(plain <"$work/texts") | awk -F'\t' 'length($4) >= 20' >"$work/long"
# longest first: at one place in an input, rg reports the first listed text that matches there
cut -f4 "$work/long" | awk '{ print length, $0 }' | sort -rn | cut -d' ' -f2- | uniq >"$work/bodies"
# each text found, under the issue the command that sent it named
if [[ -s $work/bodies ]]; then rg -F -o -n -f "$work/bodies" "$work/written" >"$work/found" || [[ $? == 1 ]]; else : >"$work/found"; fi
wait "$printing"
jq -nc --rawfile long "$work/long" --rawfile found "$work/found" --rawfile where "$work/where" --slurpfile printed "$work/printed" \
	--slurpfile comments "$work/comments" --slurpfile filed "$work/filed" '
	def rows($text): $text | split("\n") | map(select(. != ""));
	(rows($where)) as $where
	| (rows($found) | map(capture("^(?<line>[0-9]+):(?<text>.*)$") | {where: $where[(.line | tonumber) - 1], text})) as $found
	| [rows($long)[] | split("\t") as [$kind, $id, $to, $text]
		| select(any($found[]; .text == $text and (.where | IN(if $kind == "comments" then ($to, "#\($id)", "*") else $to end))))
		| {($kind): ($id | tonumber)}] as $matched
	| {comments: ([$matched[].comments // empty] + [$printed[0].comments[] | select(IN($comments[0][].id))] | unique),
	   issues: ([$matched[].issues // empty] + [$printed[0].issues[] | select(IN($filed[0][].number))] | unique)}'
