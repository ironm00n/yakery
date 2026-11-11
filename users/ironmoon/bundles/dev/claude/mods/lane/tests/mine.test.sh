#!/usr/bin/env bash
# usage: bash tests/mine.test.sh — hooks/mine.sh against a fake tracker and a session that posts the ways lanes do
set -euo pipefail
mine=$(realpath "$(dirname "$0")/../hooks/mine.sh")
source "$(dirname "$0")/fake-tracker.sh"
fail() { printf 'FAIL %s\n  want %s\n  got  %s\n' "$1" "$2" "$3"; exit 1; }

# The session: a note posted from a jq-built body, printing only the status (run B7); a note written to a file and
# posted from it, one written by a heredoc and posted from it, and one sent on stdin; one to a URL built from
# variables ($api/$n/comments) and one from Python; one command that files an issue and posts to it and to #41 (B12); a body with JSON's \n in the command; a post that printed its comment's URL; an edit (PATCH) holding the comment's new text. Not its own: a
# command quoting part of dispatch's comment, a message to a peer quoting another comment whole, the words of its post
# on issue 7 that someone also wrote on issue 9, a comment whose edit command it only wrote into a file, and the id a
# post to another forge printed. Comments too short to be told are in its commands too: "the number"
# inside the jq-built note, "echo ok" on its own
call() { jq -nc --arg id "$1" --arg name "$2" --argjson input "$3" '{type: "assistant", timestamp: "2026-11-11T04:10:00.000Z", message: {role: "assistant", content: [{type: "tool_use", id: $id, name: $name, input: $input}]}}'; }
said() { jq -nc --arg id "$1" --arg text "$2" '{type: "user", timestamp: "2026-11-11T04:10:01.000Z", message: {role: "user", content: [{type: "tool_result", tool_use_id: $id, content: $text}]}}'; }
{
	echo '{"type": "user", "timestamp": "2026-11-11T04:00:00.000Z", "origin": {"kind": "human"}, "message": {"role": "user", "content": "go"}}'
	call t1 Bash "$(jq -n --arg c "body=\$(jq -n --arg b 'Status: step \"read 1-5000\" is done. The number is 76.' '{body:\$b}') && curl -s -o /dev/null -w '%{http_code}' -X POST -d \"\$body\" \$api/issues/41/comments" '{command: $c}')"
	said t1 201
	call t2 Write '{"file_path": "/w/note.md", "content": "Handoff: the next step is the push.\nThen wait for him."}'
	said t2 'File created'
	call t3 Bash '{"command": "curl -s -o /dev/null -w %{http_code} -X POST --data-binary @/w/note.md $api/issues/41/comments"}'
	said t3 201
	call t4 Bash '{"command": "curl -s -X POST $api/issues/7/comments -d '"'"'{\"body\": \"line one\\nline two of the note\"}'"'"'"}'
	said t4 201
	call t5 Bash '{"command": "curl -s -X POST $api/issues/9/comments -d @c | jq -r .html_url"}'
	said t5 'https://code.ironmoon.dev/agents/work/issues/9#issuecomment-1990'
	call t6 Bash '{"command": "rg \"start with the step\" notes/ || echo ok"}'
	said t6 ok
	call t7 Bash '{"command": "cc-peer.py send desktop dispatch <<'"'"'EOF'"'"'\nyou wrote: Owen wants the bar on the left, not the right.\nEOF"}'
	said t7 sent
	call t8 Bash '{"command": "curl -s -o /dev/null -X POST $api/issues/7/comments -d '"'"'{\"body\": \"Same words as on nine, posted on seven.\"}'"'"'"}'
	said t8 ''
	call t9 Bash '{"command": "cat >/tmp/n.md <<'"'"'EOF'"'"'\nWritten by a heredoc, then posted from the file.\nEOF"}'
	said t9 ''
	call t10 Bash '{"command": "jq -Rs '"'"'{body: .}'"'"' /tmp/n.md | curl -s -o /dev/null -X POST $api/issues/41/comments --data-binary @-"}'
	said t10 ''
	call t11 Bash '{"command": "curl -s -o /dev/null -X POST $api/issues/41/comments -H '"'"'Content-Type: application/json'"'"' -d @- <<'"'"'EOF'"'"'\n{\"body\": \"Sent on stdin from a heredoc in the same command.\"}\nEOF"}'
	said t11 ''
	call t12 Bash '{"command": "curl -s -X PATCH $api/issues/comments/1995 -d '"'"'{\"body\": \"Edited: the push is done and the count is 76.\"}'"'"' -o /dev/null -w %{http_code}"}'
	said t12 200
	call t14 Bash '{"command": "api=https://t/api/v1/repos/agents/work/issues; n=41; jq -n --arg b '"'"'Posted where the URL is built from variables.'"'"' '"'"'{body: $b}'"'"' | curl -s -H '"'"'Content-Type: application/json'"'"' --data @- $api/$n/comments | jq -r .id"}'
	said t14 77
	call t15 Bash '{"command": "python3 - <<'"'"'EOF'"'"'\nimport json, urllib.request\ndef call(method, path, body): ...\ncall(\"POST\", \"%d/comments\" % 41, {\"body\": \"Posted from Python, with no curl at all.\"})\nEOF"}'
	said t15 ''
	call t16 Bash '{"command": "curl -s -X POST https://api.github.com/repos/o/r/issues/3/comments -d @gh.json | jq '"'"'{id}'"'"'"}'
	said t16 '{"id": 6759541583}'
	call t17 Bash '{"command": "curl -s -o /dev/null -w %{http_code} -X POST $api/issues -d '"'"'{\"title\": \"Hold the count\", \"body\": \"Closes with: the user asks for the count and gets it.\"}'"'"'"}'
	said t17 201
	call t18 Bash '{"command": "curl -s -X POST $api/issues -d @i.json | jq -r .html_url"}'
	said t18 'https://code.ironmoon.dev/agents/work/issues/79'
	call t19 Bash "$(jq -n --arg c 'N=$(jq -n '"'"'{title:"Keep the count", body:"Filed and noted in one command, as run B12 did."}'"'"' | curl -s -X POST "$API/issues" -d @- | jq -r .number)
jq -n '"'"'{body:"The note on the issue just filed, posted to it by a variable."}'"'"' | curl -s -X POST "$API/issues/$N/comments" -d @- | jq -r .id
jq -n '"'"'{body:"And the step logged on forty one in the same command."}'"'"' | curl -s -X POST "$API/issues/41/comments" -d @- | jq -r .id' '{command: $c}')"
	said t19 $'80\n1914\n1915'
	call t13 Bash '{"command": "printf '"'"'%s\\n'"'"' '"'"'curl -X PATCH $api/issues/comments/1996 -d @x'"'"' >>tests/fixture.txt"}'
	said t13 ''
} >"$dir/s1.jsonl"

jq -nc '[{id: 1900, issue: 41, body: "lt: start with the step in your first prompt."},
	{id: 1901, issue: 41, body: "Status: step \"read 1-5000\" is done. The number is 76."},
	{id: 1902, issue: 41, body: "Handoff: the next step is the push.\nThen wait for him."},
	{id: 1903, issue: 41, body: "the number"},
	{id: 1904, issue: 7, body: "line one\nline two of the note"},
	{id: 1905, issue: 41, body: "Status: step done."},
	{id: 1906, issue: 41, body: "echo ok"},
	{id: 1907, issue: 12, body: "Owen wants the bar on the left, not the right."},
	{id: 1908, issue: 9, body: "Same words as on nine, posted on seven."},
	{id: 1909, issue: 7, body: "Same words as on nine, posted on seven."},
	{id: 1910, issue: 41, body: "Written by a heredoc, then posted from the file."},
	{id: 1911, issue: 41, body: "Sent on stdin from a heredoc in the same command."},
	{id: 1995, issue: 41, body: "Edited: the push is done and the count is 76."},
	{id: 1912, issue: 41, body: "Posted where the URL is built from variables."},
	{id: 1913, issue: 41, body: "Posted from Python, with no curl at all."},
	{id: 1990, issue: 9, body: "Its text is in a file the session got from elsewhere."},
	{id: 1914, issue: 80, body: "The note on the issue just filed, posted to it by a variable."},
	{id: 1915, issue: 41, body: "And the step logged on forty one in the same command."},
	{id: 1996, issue: 41, body: "Someone else'"'"'s comment, whose id a command only wrote down."}]
	| map(. + {user: {login: "agent"}, issue_url: "https://t/api/v1/repos/agents/work/issues/\(.issue)"} | del(.issue))' |
	api 'issues_comments_since_2026-11-11T04%3A00%3A00.000Z_before_2026-11-11T04%3A10%3A01.000Z_limit_50_page_1'
jq -nc '[{number: 77, body: "Closes with: the user asks for the count and gets it."},
	{number: 78, body: "Someone else filed this one, about something else entirely."},
	{number: 79, body: "Its text is in a file the session got from elsewhere."},
	{number: 80, body: "Filed and noted in one command, as run B12 did."}]' |
	api 'issues_state_all_type_issues_since_2026-11-11T04%3A00%3A00.000Z_before_2026-11-11T04%3A10%3A01.000Z_limit_50_page_1'
run() { HOME=$dir/home PATH=$dir/bin:$PATH FAKE_API=$dir/api LANE_TRACKER=https://t bash "$mine" "$1" agents/work; }

got=$(run "$dir/s1.jsonl" | jq -c .comments)
want='[1901,1902,1904,1909,1910,1911,1912,1913,1914,1915,1990,1995]'
[[ $got == "$want" ]] || fail "the comments whose whole text the session posted to their issue, and those whose ids it printed or edited, are its own" "$want" "$got"
echo "ok: a lane's comments are found by their text in what it posted to each issue, and by the ids it printed or edited"
got=$(run "$dir/s1.jsonl" | jq -c .issues)
[[ $got == '[77,79,80]' ]] || fail "the issues whose text the session sent to file one, and the one whose URL it printed, are its own" '[77,79,80]' "$got"
echo "ok: the issues a lane filed are found the same way"
[[ $(grep -c '^issues_comments' "$dir/requests") == 2 ]] && grep -q 'issues_comments_since_2026-11-11T04%3A00%3A00.000Z_before_2026-11-11T04%3A10%3A01.000Z_limit_50_page_1' "$dir/requests" ||
	fail "the comments are listed from the session's first row to its last, a short page the last" 'one request, since and before' "$(cat "$dir/requests")"
echo 'ok: only comments made while the session ran are asked for'
listing=issues_comments_since_2026-11-11T04%3A00%3A00.000Z_before_2026-11-11T04%3A10%3A01.000Z_limit_50_page_1
cp "$dir/api/$listing" "$dir/listing" && jq -c 'map(del(.issue_url))' "$dir/listing" >"$dir/api/$listing"
if run "$dir/s1.jsonl" >/dev/null 2>&1; then fail 'comments that say no issue fail the run, not drop out' failed ran; fi
cp "$dir/listing" "$dir/api/$listing"
echo 'ok: a listing it cannot read fails the run rather than finding nothing'
echo "$listing" >"$dir/api/fail"
if run "$dir/s1.jsonl" >/dev/null 2>&1; then fail 'a tracker that fails fails the run' failed ran; fi
echo 'ok: a tracker that fails fails the run, so the context is read again at the next kit'
