#!/usr/bin/env bash
# usage: bash tests/commits.test.sh — hooks/commits.sh over scratch jj and git repos with remotes, a second jj
# workspace, a git repo with none, and transcripts that work in them
set -euo pipefail
commits=$(realpath "$(dirname "$0")/../hooks/commits.sh")
dir=$(mktemp -d) && trap 'rm -rf "$dir"' EXIT
export HOME=$dir/home JJ_CONFIG=$dir/jj.toml GIT_CONFIG_GLOBAL=$dir/gitconfig GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME" "$dir/plain" "$dir/bin"
printf '[user]\nname = "t"\nemail = "t@t"\n' >"$JJ_CONFIG"
printf '[user]\n\tname = t\n\temail = t@t\n[init]\n\tdefaultBranch = main\n' >"$GIT_CONFIG_GLOBAL"
q() { "$@" >/dev/null 2>&1; }

# jj, colocated: base on main and lane work on feat, both pushed; a scratch commit abandoned; ten commits above them
# unpushed; @ empty; a second workspace jw2 beside it, and a third jw3 on main
jw=$dir/jw && q jj git init --colocate "$jw" && cd "$jw"
q git init --bare "$dir/jw-remote.git" && q jj git remote add origin "$dir/jw-remote.git"
short() { jj --ignore-working-copy log --no-graph -r "$1" -T 'commit_id.short(8)'; }
q jj commit -m base && q jj bookmark set main -r @- && q jj git push -b main && base=$(short @-)
q jj commit -m 'lane work' && q jj bookmark set feat -r @- && q jj git push -b feat && work=$(short @-)
echo s >s && q jj commit -m scratch && scratch=$(short @-) && q jj abandon -r @-
for i in $(seq 1 10); do echo "$i" >"f$i" && q jj commit -m "unpushed $i"; done
unpushed5=$(short 'description(substring:"unpushed 5")')
q jj -R "$jw" workspace add "$dir/jw2" && q jj -R "$jw" workspace add -r main "$dir/jw3" && working_copy=$(short @)
mkdir sub && touch untracked
ops() { jj --ignore-working-copy -R "$jw" op log --no-graph -T 'id ++ "\n"' | wc -l; }
ops_before=$(ops)

# git: g1 pushed and tracked; a draft amended into g2, local; one file changed in the working tree
gw=$dir/gw && q git init "$gw" && cd "$gw"
q git init --bare "$dir/gw-remote.git" && q git remote add origin "$dir/gw-remote.git"
echo 1 >a && q git add a && q git commit -m g1 && q git push -u origin main
echo 2 >b && q git add b && q git commit -m g2-draft && draft=$(git rev-parse --short HEAD) && q git commit --amend -m g2
echo 3 >a && touch -d '+1 minute' b
g1=$(git rev-parse --short HEAD~) g2=$(git rev-parse --short HEAD)
index() { stat -c %y "$gw/.git/index"; }
index_before=$(index)

# git pushed with no upstream set and no origin/HEAD: n1 on origin/main, n2 local
gn=$dir/gn && q git init "$gn" && q git init --bare "$dir/gn-remote.git" && q git -C "$gn" remote add origin "$dir/gn-remote.git"
echo 1 >"$gn/a" && q git -C "$gn" add a && q git -C "$gn" commit -m n1 && q git -C "$gn" push origin main
echo 2 >"$gn/a" && q git -C "$gn" commit -am n2

# git with no remote: two commits
gl=$dir/gl && q git init "$gl" && cd "$gl"
echo 1 >a && q git add a && q git commit -m l1 && echo 2 >a && q git commit -am l2
l1=$(git rev-parse --short HEAD~) l2=$(git rev-parse --short HEAD)
cd "$dir"

row() { jq -cn --arg cwd "$1" --arg id "$2" --arg command "$3" --arg out "$4" '
	{type: "assistant", cwd: $cwd, message: {content: [{type: "tool_use", id: $id, name: "Bash", input: {$command}}]}},
	{type: "user", cwd: $cwd, message: {content: [{type: "tool_result", tool_use_id: $id, content: $out}]}}'; }
# the session looked at jw2, committed from below jw's root, committed in gw and landed from a directory that is no
# repository; its output named every commit above and an id of none
{
	row "$dir" t0 "jj -R $dir/jw2 log" "nothing named"
	row "$jw/sub" t1 "jj commit -m 'lane work'" "Parent commit (@-): $work lane work; abandoned $scratch; Working copy now at: $working_copy"
	row "$dir" t2 "git -C $gw commit -m g2" "[main $g2] g2, amending $draft, after $g1"
	row "$dir" t3 "cd $dir/plain && ~/workspace/claude/tools/land x" "land: x is on master at $base; deadbeef"
} >"$dir/transcript.jsonl"
row "$gl" t1 'git commit -m l2' "[main $l2] l2, after $l1" >"$dir/local.jsonl"
out=$(bash "$commits" "$dir/plain" "$dir/transcript.jsonl")
ws() { jq -c --arg root "$1" ".[] | select(.root == \$root) | $2" <<<"$out"; }

fails=0
check() {
	if [[ $2 == "$3" ]]; then echo "ok: $1"; else printf 'FAIL: %s\n  want %s\n  got  %s\n' "$1" "$3" "$2"; fails=1; fi
}
check 'the repositories the session named, newest first, then those of its cwds, a jj one found from below its root' \
	"$(jq -c 'map([.root, .vcs])' <<<"$out")" "[[\"$gw\",\"git\"],[\"$dir/jw2\",\"jj\"],[\"$jw\",\"jj\"]]"
check 'jj: the working copy, empty' "$(ws "$jw" '.head | [.subject, .empty]')" '["",true]'
check 'jj: the newest ten commits below it that trunk lacks, unpushed, the rest counted' \
	"$(ws "$jw" '[(.line | map(.subject)), (.line | map(.pushed) | unique), .more]')" \
	'[["unpushed 10","unpushed 9","unpushed 8","unpushed 7","unpushed 6","unpushed 5","unpushed 4","unpushed 3","unpushed 2","unpushed 1"],[false],1]'
check 'jj: of the commits the output named, the one no line holds, in the first workspace listing it, with its bookmarks' \
	"$(ws "$dir/jw2" '.named | map([.subject, .bookmarks, .pushed])') $(ws "$jw" '.named')" '[["base",["main"],true]] []'
{
	row "$dir" t4 "jj -R $jw log" "nothing named"
	row "$dir" t5 "jj -R $dir/jw3 commit -m x" "named $unpushed5"
} >"$dir/line.jsonl"
check 'a named commit that a workspace listed after holds in its line is left to that line' \
	"$(bash "$commits" "$dir/plain" "$dir/line.jsonl" | jq -c 'map([(.root | sub(".*/"; "")), (.named | length)])')" '[["jw3",0],["jw",0]]'
check 'jj: change and commit ids, and commit times in UTC' \
	"$(ws "$dir/jw2" '.named[0] | [(.change | test("^[k-z]{8}$")), .commit, (.at | test("^\\d{4}-\\d\\d-\\d\\dT\\d\\d:\\d\\d:\\d\\dZ$"))]')" "[true,\"$base\",true]"
check 'git: HEAD, unpushed, on its branch, and the files changed in the working tree' \
	"$(ws "$gw" '[.head.subject, .head.bookmarks, .head.pushed, .changed]')" '["g2",["main"],false,1]'
check 'git: nothing else below it that its upstream lacks' "$(ws "$gw" '[.line, .more]')" '[[],0]'
check 'git: of the commits the output named, the one below neither, and not the draft no branch leads to' \
	"$(ws "$gw" '.named | map([.commit, .subject, .pushed])')" "[[\"$g1\",\"g1\",true]]"
check 'jj was only read: no operation, no snapshot of the untracked file' "$(ops)" "$ops_before"
check 'git was only read: the index is as it was' "$(index)" "$index_before"
check 'git with no upstream: every commit below HEAD, so none counts as only named' \
	"$(bash "$commits" "$gl" "$dir/local.jsonl" | jq -c '.[0] | [.head.subject, (.line | map(.subject)), .more, .named]')" '["l2",["l1"],0,[]]'
printf '#!/usr/bin/env bash\n[[ $* != *") | commit_id("* ]] || { echo "Error: Commit ID prefix is ambiguous" >&2; exit 1; }\nexec %q "$@"\n' "$(command -v jj)" >"$dir/bin/jj"
chmod +x "$dir/bin/jj"
check 'jj: when one prefix fails the query of all ids, each is asked alone' \
	"$(PATH=$dir/bin:$PATH bash "$commits" "$dir/plain" "$dir/transcript.jsonl" | jq -c 'map(.named // [] | map(.subject))')" '[["g1"],["base"],[]]'
printf '#!/usr/bin/env bash\n[[ $* != *" log "* ]] || { echo "Error: The working copy is stale" >&2; exit 1; }\nexec %q "$@"\n' "$(command -v jj)" >"$dir/bin/jj"
check 'a workspace that cannot be listed says why, and the others are listed' \
	"$(PATH=$dir/bin:$PATH bash "$commits" "$dir/plain" "$dir/transcript.jsonl" | jq -c 'map(.error // .head.subject)')" \
	'["g2","Error: The working copy is stale","Error: The working copy is stale"]'
check "git with no upstream: trunk is the remote's main, as jj's trunk() would take it" \
	"$(bash "$commits" "$gn" | jq -c '.[0] | [.head.subject, .line, .more]')" '["n2",[],0]'
check 'with no transcript, the cwd alone' "$(bash "$commits" "$gw" | jq -c 'map(.root)')" "[\"$gw\"]"
check 'a cwd that is no repository, and no transcript, gives none' "$(bash "$commits" "$dir/plain")" '[]'
q jj -R "$jw" log && q git -C "$gw" status
check '… where a plain jj log and git status do write' "$( [[ $(ops) != "$ops_before" ]] && echo op) $( [[ $(index) != "$index_before" ]] && echo index)" 'op index'
exit $fails
