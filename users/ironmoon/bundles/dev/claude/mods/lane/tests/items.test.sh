#!/usr/bin/env bash
# usage: bash tests/items.test.sh — hooks/items.sh against a fake tracker
set -euo pipefail
items=$(realpath "$(dirname "$0")/../hooks/items.sh")
dir=$(mktemp -d) && trap 'rm -rf "$dir"' EXIT
token=$dir/home/.claude/secrets/forgejo-agent-pat
mkdir -p "$(dirname "$token")" "$dir/bin" "$dir/api"
echo test-token >"$token"
# The fake tracker. A request's answer is api/<the URL past the repo, with / ? & = as _>. With no such file, a listing of
# issues under a label answers every issue on its first page, as the real tracker does for a label it lacks, and anything
# else answers an empty listing. A request named in api/fail fails. Each request's URL and arguments are logged.
cat >"$dir/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_API/../args"
config=$( [[ " $* " == *" -K - "* ]] && cat )
[[ $config == 'header = "Authorization: token test-token"' ]] || exit 22
name=$(sed 's|^https://t/||; s|[/?&=]|_|g' <<<"${!#}")
echo "$name" >>"$FAKE_API/../requests"
! grep -qx "$name" "$FAKE_API/fail" 2>/dev/null || exit 22
if [[ -f $FAKE_API/$name ]]; then cat "$FAKE_API/$name"
elif [[ $name == issues_state_open_type_issues_labels_*_page_1 ]]; then cat "$FAKE_API/every-issue"
else echo '[]'; fi
EOF
chmod +x "$dir/bin/curl"
api() { cat >"$dir/api/$1"; }
issue() {
	jq -cn --argjson n "$1" --arg u "$2" --argjson c "$3" --args \
		'{number: $n, title: "t\($n)", html_url: "https://w/\($n)", updated_at: $u, comments: $c, body: "b\($n)", labels: [$ARGS.positional[] | {name: .}]}' "${@:4}"
}
api labels__limit_50_page_1 <<<'[{"name":"owner/lane"},{"name":"owner:lane"},{"name":"owner/other"},{"name":"owner/lane-2"}]'
jq -s . <(issue 5 2026-10-02 2 owner/lane) <(issue 7 2026-10-03 0 owner/lane owner:lane) <(issue 8 2026-10-01 1 owner:lane) <(issue 9 2026-10-04 0 owner/other) |
	api every-issue
# the filter on owner/lane also answers #9, which the lane does not own
jq -s . <(issue 5 2026-10-02 2 owner/lane) <(issue 7 2026-10-03 0 owner/lane owner:lane) <(issue 9 2026-10-04 0 owner/other) |
	api issues_state_open_type_issues_labels_owner%2Flane_limit_50_page_1
jq -s . <(issue 7 2026-10-03 0 owner/lane owner:lane) <(issue 8 2026-10-01 1 owner:lane) |
	api issues_state_open_type_issues_labels_owner%3Alane_limit_50_page_1
api issues_5_comments <<<'[{"user":{"login":"a"},"created_at":"2026-10-01","body":"older"},{"user":{"login":"dispatch"},"created_at":"2026-10-02","body":"newest"}]'
api issues_8_comments <<<'[{"user":{"login":"b"},"created_at":"2026-10-01","body":"only"}]'
run() { HOME=$dir/home PATH=$dir/bin:$PATH FAKE_API=$dir/api LANE_TRACKER=https://t bash "$items" "$1" agents/work; }
fails=0
check() {
	if [[ $2 == "$3" ]]; then echo "ok: $1"; else printf 'FAIL: %s\n  want %s\n  got  %s\n' "$1" "$3" "$2"; fails=1; fi
}
check 'owned issues only, once each, most recently updated first' "$(run lane | jq -c 'map(.number)')" '[7,5,8]'
check 'each with its newest comment' "$(run lane | jq -c 'map(.newest)')" \
	'[null,{"by":"dispatch","at":"2026-10-02","body":"newest"},{"by":"b","at":"2026-10-01","body":"only"}]'
check 'the fields the kit reads' "$(run lane | jq -c '.[0] | del(.newest)')" \
	'{"number":7,"title":"t7","url":"https://w/7","updated":"2026-10-03","labels":["owner/lane","owner:lane"],"body":"b7"}'
: >"$dir/requests"
check 'a lane with no label owns nothing' "$(run nobody)" '[]'
check '… and no issues are asked for under a label the tracker lacks' "$(grep -c '^issues' "$dir/requests" || true)" 0
check 'the token is on no command line' "$(grep -c test-token "$dir/args" || true)" 0
echo issues_5_comments >"$dir/api/fail"
check 'a failed request prints nothing' "$(run lane 2>/dev/null || true)" ''
check '… and fails the run' "$(if run lane >/dev/null 2>&1; then echo ran; else echo failed; fi)" failed
rm "$dir/api/fail"
echo wrong >"$token"
check 'a tracker refusing the token fails the run' "$(if run lane >/dev/null 2>&1; then echo ran; else echo failed; fi)" failed
echo test-token >"$token"
: >"$dir/args"
HOME=$dir/home PATH=$dir/bin:$PATH FAKE_API=$dir/api bash "$items" lane some/repo >/dev/null
check 'with no LANE_TRACKER the issues are read from the repository named on the forge' "$(head -1 "$dir/args" | grep -o 'https://[^ ]*')" \
	'https://code.ironmoon.dev/api/v1/repos/some/repo/labels?&limit=50&page=1'
exit $fails
