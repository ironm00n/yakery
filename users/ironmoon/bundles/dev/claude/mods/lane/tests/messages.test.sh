#!/usr/bin/env bash
# usage: bash tests/messages.test.sh — hooks/messages.jq over each tests/messages-<case>.jsonl against its .expected.json,
# with the caps in its .caps if it has one and none binding otherwise: walk (a rewind, a sidechain, queued, peer and
# subagent rows, a compaction, a torn last line), loop (a compaction's link that leads back under it), dangling (a
# compaction's link that names no row), bounds (the newest that fit each section, cut and counted, empty ones bounded)
set -euo pipefail
here=$(dirname "$0") fails=0
for input in "$here"/messages-*.jsonl; do
	case=$(basename "$input" .jsonl)
	caps=$(cat "${input%.jsonl}.caps" 2>/dev/null || echo '{"owen": 1e9, "peers": 1e9, "message": 1e9}')
	if diff <(jq -cnR --argjson caps "$caps" -f "$here/../hooks/messages.jq" "$input") "${input%.jsonl}.expected.json"; then
		echo "ok: $case"
	else
		echo "FAIL: $case"
		fails=1
	fi
done
exit $fails
