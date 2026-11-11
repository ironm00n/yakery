#!/usr/bin/env bash
# usage: bash tests/jq.test.sh — each tests/<program>-<case>.jsonl through hooks/<program>.jq against the value in its
# .expected.json, its jq arguments the JSON object in its .args, or else its program's defaults below.
#   messages: walk (a rewind, a sidechain, queued, peer and subagent rows, a compaction, a torn last line), loop (a
#     compaction's link that leads back under it), dangling (a compaction's link that names no row), bounds (the newest
#     that fit each section, cut and counted, empty ones bounded)
#   worked: paths (each kind of directory and id, newest first and the cwds after the rest, quoted, relative and ~
#     paths, a heredoc's body, prose, flags and variables left out,
#     reads and other commands' output left out, a torn last line), bounds (only the newest 32 directories and 64 ids)
#   posted: calls (the ids of comments, and the numbers of issues, a session's commands posted, from the URL or the
#     JSON each printed; reads, bare numbers, an edit that printed none, subagents' calls and a torn last line left out)
#   tail: rules (each kind of row, the lane mod's own rows and what came after the ask left out, a long result cut, a torn
#     last line), window (the last prompts and what follows them), levels (without results when over the budget), cut
#     (the newest characters when nothing fits)
set -euo pipefail
here=$(dirname "$0") fails=0
declare -A defaults=([messages]='{"caps": {"owen": 1e9, "peers": 1e9, "message": 1e9}}' [worked]='{"home": "/home/u"}' [posted]='{}')
for input in "$here"/*-*.jsonl; do
	case=$(basename "$input" .jsonl) program=${case%%-*}
	mapfile -t args < <(jq -r 'to_entries[] | "--argjson", .key, (.value | tojson)' <(cat "${input%.jsonl}.args" 2>/dev/null || echo "${defaults[$program]}"))
	if diff <(jq -cnR "${args[@]}" -f "$here/../hooks/$program.jq" "$input" | jq .) <(jq . "${input%.jsonl}.expected.json"); then
		echo "ok: $case"
	else
		echo "FAIL: $case"
		fails=1
	fi
done
exit $fails
