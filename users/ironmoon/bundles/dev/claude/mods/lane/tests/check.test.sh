#!/usr/bin/env bash
# usage: bash tests/check.test.sh — hooks/check.sh on scratch rotations, with a claude that answers each stage from the
# replies below and keeps what it was given, and a claude-pool-now that reads pool.json
set -euo pipefail
check=$(realpath "$(dirname "$0")/../hooks/check.sh")
dir=$(mktemp -d) && trap 'rm -rf "$dir"' EXIT
mkdir -p "$dir/bin"
fail() { printf 'FAIL %s\n  want %s\n  got  %s\n' "$1" "$2" "$3"; exit 1; }
OPUS=claude-opus-5-5 SONNET=claude-sonnet-5-5

cat >"$dir/bin/claude" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
dir=$(dirname "$(dirname "$(realpath "$0")")")
args=("$@") model=
for ((i = 0; i < ${#args[@]}; i++)); do [[ ${args[i]} == --model ]] && model=${args[i + 1]}; done
prompt=$(cat)
case $prompt in *'# TRANSCRIPT'*) stage=extract ;; *'<material>'*) stage=answer ;; *'# ANSWERS X'*) stage=judge ;; *) stage=unknown ;; esac
call=$dir/calls/$stage.$model
mkdir -p "$dir/calls"
printf '%s\n' "$@" >"$call.argv"
env | cut -d= -f1 | sort >"$call.env"
printf '%s' "$prompt" >"$call.prompt"
if [[ -e $dir/fail.$stage.$model ]]; then
	jq -n '{is_error: true, result: "API Error: 529 overloaded", total_cost_usd: 0, modelUsage: {}}'
	exit 1
fi
jq -n --rawfile r "$dir/reply.$stage.$model" --arg m "$model" '{is_error: false, result: ("Here:\n```json\n" + $r + "```"), total_cost_usd: 0.25, modelUsage: {($m): {}}}'
EOF
printf '#!/usr/bin/env bash\ncat "%s/pool.json"\n' "$dir" >"$dir/bin/claude-pool-now"
chmod +x "$dir/bin/claude" "$dir/bin/claude-pool-now"
pool() { jq -n --argjson out "$1" '{at: 1000, users: [{name: "owen", avail: 3, runway: {out: $out}}]}' >"$dir/pool.json"; }

# The cleared context: Owen's ask, the session's answer, the mod's ask to start over, and the notes the session then left
# on its issue and in its state file, which the extractor must not see
cat >"$dir/s1.jsonl" <<'EOF'
{"type": "user", "uuid": "u1", "timestamp": "2026-11-11T04:00:00.000Z", "origin": {"kind": "human"}, "message": {"role": "user", "content": "count the 777 lines and keep the number"}}
{"type": "assistant", "uuid": "a1", "timestamp": "2026-11-11T04:00:01.000Z", "message": {"role": "assistant", "content": [{"type": "text", "text": "76 lines contain 777."}]}}
{"type": "attachment", "uuid": "k1", "timestamp": "2026-11-11T04:05:00.010Z", "attachment": {"type": "hook_additional_context", "hookName": "PostToolUse:Bash", "content": ["You landed work this turn: start over"]}}
{"type": "assistant", "uuid": "a2", "timestamp": "2026-11-11T04:05:01.000Z", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "curl -X POST https://t/api/v1/repos/agents/work/issues/41/comments -d @note"}}]}}
{"type": "user", "uuid": "u2", "timestamp": "2026-11-11T04:05:02.000Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "{\"body\": \"ISSUE NOTE: the count is 76\"}"}]}}
{"type": "assistant", "uuid": "a3", "timestamp": "2026-11-11T04:05:03.000Z", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "t2", "name": "Edit", "input": {"file_path": "/w/lanes/lt.md"}}]}}
{"type": "user", "uuid": "u3", "timestamp": "2026-11-11T04:05:04.000Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t2", "content": "updated: STATE NOTE count 76"}]}}
EOF
: >"$dir/empty.jsonl"
KIT='KIT: issue 41, newest comment: the count is 76'

jq -nc '[range(1; 17) | {q: ., question: "Q\(.)?", answer: "R\(.)", where: "[2026-11-11 04:00Z] OWEN"}]' >"$dir/reply.extract.$OPUS"
for model in $OPUS $SONNET; do jq -nc --arg m "$model" '[range(1; 17) | {q: ., answer: "\($m) says A\(.)"}]' >"$dir/reply.answer.$model"; done
jq -nc '[range(1; 17) | {q: ., X: "correct", Y: (if . <= 8 then "partial" else "not stated" end), note: ""}]' >"$dir/reply.judge.$OPUS"

# rotation TO TRANSCRIPT [CLAUDE]: the check's directory for the rotation from s1 to TO, as the mod leaves it
rotation() {
	local d=$dir/checks/lt/$1
	mkdir -p "$d"
	printf '%s' "$KIT" >"$d/kit.txt"
	jq -n --arg to "$1" --arg transcript "$2" --arg claude "${3-$dir/bin/claude}" --arg results "$dir/checks/lt.jsonl" '{lane: "lt", arm: "B", from: "s1", $to,
		$transcript, notes: ["/w/lanes/lt.md"], tracker: "agents/work", at: "2026-11-11T04:06:00.000Z", $claude, $results} | if $claude == "" then del(.claude) end' >"$d/rotation.json"
	echo "$d"
}
run() {
	rm -rf "$dir/calls" "$dir/checks/lt.jsonl"
	PATH=$dir/bin:$PATH CLAUDECODE=1 CLAUDE_CODE_SESSION_ID=s1 CLAUDE_JOB_DIR=/j CLAUDE_CONFIG_DIR=$dir/config HTTPS_PROXY=http://tap bash "$check" "$1"
	tail -1 "$dir/checks/lt.jsonl"
}
same() { jq -e --argjson want "$2" '. == $want' <<<"$3" >/dev/null || fail "$1" "$2" "$3"; }
grades() { jq -nc --argjson c "$1" --argjson p "$2" --argjson n "$3" '{correct: $c, partial: $p, wrong: 0, "not stated": $n, score: ($c + $p / 2)}'; }
FULL=$(grades 16 0 0) HALF=$(grades 0 8 8)
done_row() { jq -nc --arg dir "$1" --argjson sonnet "$2" --argjson opus "$3" '{lane: "lt", arm: "B", from: "s1", to: ($dir | split("/") | last),
	at: "2026-11-11T04:06:00.000Z", status: "done", questions: 16, $sonnet, $opus, cost: 1, $dir}'; }
prompt() { cat "$dir/calls/$1.prompt"; }
has() { [[ $3 == *"$2"* ]] || fail "$1" "it to hold $2" "${3:0:300}"; }
lacks() { [[ $3 != *"$2"* ]] || fail "$1" "it not to hold $2" "${3:0:300}"; }

pool null
# s2's SHA-256 begins ad, an odd byte, so the judge reads Opus's answers as X; t2's begins c4, so Sonnet's
d=$(rotation s2 "$dir/s1.jsonl")
same 'a check row counts each model by grade' "$(done_row "$d" "$HALF" "$FULL")" "$(run "$d")"
extract=$(prompt "extract.$OPUS")
has 'the extractor reads the cleared context' 'count the 777 lines and keep the number' "$extract"
has '… asks for 16 questions' 'Write 16 questions' "$extract"
lacks '… but not the note it left on its issue' 'ISSUE NOTE' "$extract"
lacks '… nor the one in its state file' 'STATE NOTE' "$extract"
lacks '… nor the kit' 'KIT:' "$extract"
for model in $OPUS $SONNET; do
	answer=$(prompt "answer.$model")
	has "$model answers from the kit" "$KIT" "$answer"
	has "… all 16 questions" '16. Q16?' "$answer"
	lacks '… without the reference answers' 'R16' "$answer"
done
judge=$(prompt "judge.$OPUS")
has 'the judge has the references' 'Reference: R16' "$judge"
lacks '… and not the kit' 'KIT:' "$judge"
x=${judge#*'# ANSWERS X'} && x=${x%%'# ANSWERS Y'*}
has '… with Opus as X for s2' "$OPUS says A1" "$x"
for call in "$dir"/calls/*.argv; do
	same "$(basename "$call" .argv) runs with no hooks, mods, CLAUDE.md, tools or saved session" \
		'["-p","--safe-mode","--no-session-persistence","--tools","","--output-format","json"]' "$(jq -Rsc 'split("\n")[:-1] | index("--model") as $i | .[:$i] + .[$i + 2:]' "$call")"
	env=$(<"${call%.argv}.env")
	for name in HOME HTTPS_PROXY CLAUDE_CONFIG_DIR CLAUDE_CODE_NO_MODEL_FALLBACK; do has "$(basename "$call" .argv) keeps $name" "$name" "$env"; done
	for name in CLAUDECODE CLAUDE_CODE_SESSION_ID CLAUDE_JOB_DIR; do lacks "$(basename "$call" .argv) drops the session's $name" $'\n'"$name"$'\n' $'\n'"$env"$'\n'; done
done
echo 'ok: a rotation is checked, the extractor never sees the kit, the judge never the kit or the model'

d=$(rotation t2 "$dir/s1.jsonl")
same "the grades follow the judge's order for t2" "$(done_row "$d" "$FULL" "$HALF")" "$(run "$d")"
echo 'ok: the order is undone by the key'

skipped() { jq -nc --arg dir "$1" --arg why "$2" '{lane: "lt", arm: "B", from: "s1", to: ($dir | split("/") | last), at: "2026-11-11T04:06:00.000Z", status: "skipped", $why, $dir}'; }
failed() { jq -nc --arg dir "$1" --arg why "$2" '{lane: "lt", arm: "B", from: "s1", to: ($dir | split("/") | last), at: "2026-11-11T04:06:00.000Z", status: "failed", $why, $dir}'; }
pool 4000
d=$(rotation s3 "$dir/s1.jsonl")
same 'a pool that would refuse within the hour skips the check' "$(skipped "$d" 'the pool would refuse owen within an hour')" "$(run "$d")"
[[ ! -e $dir/calls ]] || fail '… with no model call' 'no calls' "$(ls "$dir/calls")"
pool 1000
same '… as does one refusing now' "$(skipped "$d" 'the pool would refuse owen within an hour')" "$(run "$d")"
pool null
echo 'ok: the pool is asked first'

d=$(rotation s4 "$dir/s1.jsonl" '')
same 'an engine whose binary was not found skips the check' "$(skipped "$d" "the engine's binary was not found, so no model call was made")" "$(run "$d")"
d=$(rotation s5 "$dir/empty.jsonl")
same 'a cleared context with nothing in it skips the check' "$(skipped "$d" 'the cleared context left nothing to ask about')" "$(run "$d")"
echo 'ok: what cannot be checked is said'

touch "$dir/fail.answer.$SONNET"
d=$(rotation s6 "$dir/s1.jsonl")
same 'a model call that fails is named' "$(failed "$d" "answer by $SONNET: API Error: 529 overloaded")" "$(run "$d")"
rm "$dir/fail.answer.$SONNET"
jq -nc '[range(1; 18) | {q: ., question: "Q\(.)?", answer: "R\(.)", where: ""}]' >"$dir/reply.extract.$OPUS"
same '… as is an extract of more than 16 questions' "$(failed "$d" 'extract: 17 questions, not 1 to 16')" "$(run "$d")"
echo '[]' >"$dir/reply.extract.$OPUS"
same '… or of none' "$(failed "$d" 'extract: 0 questions, not 1 to 16')" "$(run "$d")"
jq -nc '[range(1; 17) | {q: ., question: "Q\(.)?", answer: "R\(.)", where: ""}]' >"$dir/reply.extract.$OPUS"
jq -nc '[range(1; 17) | {q: ., X: "correct", Y: (if . == 1 then "fine" else "wrong" end), note: ""}]' >"$dir/reply.judge.$OPUS"
same '… and a judge whose grades are not one each of the four' "$(failed "$d" 'judge: not one grade of correct, partial, wrong or not stated for each answer')" "$(run "$d")"
echo 'ok: failures are named in the row'

# A context too short to hold 16 facts: the extractor writes the 10 it holds, and both models are graded out of 10
jq -nc '[range(1; 11) | {q: ., question: "Q\(.)?", answer: "R\(.)", where: ""}]' >"$dir/reply.extract.$OPUS"
jq -nc '[range(1; 11) | {q: ., X: "correct", Y: (if . <= 4 then "partial" else "not stated" end), note: ""}]' >"$dir/reply.judge.$OPUS"
d=$(rotation s2 "$dir/s1.jsonl")
same 'a short extract is graded out of the questions it holds' \
	"$(done_row "$d" "$(grades 0 4 6)" "$(grades 10 0 0)" | jq -c '.questions = 10')" "$(run "$d")"
has '… and the extractor was told it may write fewer' 'write as many as it holds' "$(prompt "extract.$OPUS")"
echo 'ok: a short context is checked on what it holds'
