#!/usr/bin/env bash
# usage: setsid -f bash hooks/check.sh DIR — the recall check of one rotation, run apart from the session. DIR holds what
# the mod left there: rotation.json (the lane, its arm, the cleared context's id and transcript, its state file's paths,
# the tracker's repository, when it rotated, the engine's binary and the results file) and kit.txt (what the fresh context was handed). An
# extractor reads the cleared context's last stretch (tail.jq), never the kit nor either arm's notes, and writes 16
# questions a successor
# needs answered, or as many as a shorter stretch holds; Sonnet and Opus answer them from the kit alone; Opus grades both sets against the extractor's answers,
# told neither the arm nor which model wrote which. One row goes to the results file, done, skipped or failed, and every
# model input and output stays in DIR.
set -uo pipefail
dir=$1
exec </dev/null >>"$dir/check.log" 2>&1
here=$(dirname "$(realpath "$0")")
opus=claude-opus-5-5 sonnet=claude-sonnet-5-5 count=16 pool_user=owen
grades='["correct", "partial", "wrong", "not stated"]'
rotation=$(<"$dir/rotation.json")
field() { jq -r --arg k "$1" '.[$k] // empty' <<<"$rotation"; }
claude=$(field claude) results=$(field results) lane=$(field lane)

row() {
	jq -c --arg status "$1" --arg dir "$dir" --argjson more "$2" '{lane, arm, from, to, at, $status} + $more + {$dir}' <<<"$rotation" |
		{ flock 9 && cat >>"$results"; } 9>>"$results"
}
stop() {
	row "$1" "$(jq -nc --arg why "$2" '{$why}')"
	exit 0
}

# fill TEMPLATE NAME=VALUE|NAME@FILE...: hooks/check/TEMPLATE.md with each {{NAME}} replaced by VALUE or FILE's text,
# in one pass
fill() {
	local template=$1 args=()
	shift
	for pair in "$@"; do
		local name=${pair%%[=@]*}
		local how=${pair:${#name}:1} value=${pair:${#name}+1}
		if [[ $how == = ]]; then args+=(--arg "$name" "$value"); else args+=(--rawfile "$name" "$value"); fi
	done
	jq -nr --rawfile template "$here/check/$template.md" "${args[@]}" '$ARGS.named as $values
		| [$template | splits("\\{\\{|\\}\\}")] | to_entries | map(if .key % 2 == 1 then $values[.value] // error("no \(.value)") else .value end) | join("")'
}

# ask NAME MODEL PROMPT: the PROMPT file through the engine's binary with no hooks, mods, CLAUDE.md or tools and no
# session saved, in the session's environment but for the engine's own variables, held to MODEL; its output goes to
# NAME.json, the reply's fenced JSON to NAME.reply.json, or why there is none to NAME.why
ask() {
	local name=$1 model=$2 prompt=$3
	(
		for variable in $(compgen -e); do [[ $variable == CLAUDE* && $variable != CLAUDE_CONFIG_DIR ]] && unset "$variable"; done
		export CLAUDE_CODE_NO_MODEL_FALLBACK=1
		exec timeout 900 "$claude" -p --safe-mode --no-session-persistence --tools "" --output-format json --model "$model" <"$prompt"
	) >"$dir/$name.json" 2>"$dir/$name.err"
	local code=$?
	if jq -e --arg m "$model" '.is_error == false and (.modelUsage | has($m))' "$dir/$name.json" >/dev/null 2>&1; then
		jq '.result | (capture("```json\\s*(?<j>[\\s\\S]*?)```").j // .) | fromjson' "$dir/$name.json" >"$dir/$name.reply.json" 2>"$dir/$name.why" && return 0
	else
		jq -r --arg m "$model" 'if .is_error then .result else "answered by \(.modelUsage | keys | join(", ")), not \($m)" end' "$dir/$name.json" >"$dir/$name.why" 2>/dev/null ||
			{ tail -1 "$dir/$name.err" | grep . || echo "exited $code"; } >"$dir/$name.why"
	fi
	return 1
}
why() { tr '\n' ' ' <"$dir/$1.why" | sed 's/ *$//'; }

[[ -n $claude ]] || stop skipped "the engine's binary was not found, so no model call was made"
claude-pool-now --json --user "$pool_user" >"$dir/pool.json" 2>"$dir/pool.err" || stop skipped "the pool was not read: $(tail -1 "$dir/pool.err")"
case $(jq -r --arg u "$pool_user" '.at as $at | [.users[] | select(.name == $u)][0]
	| if . == null then "unlisted" elif .runway.out == null or .runway.out - $at >= 3600 then "ok" else "soon" end' "$dir/pool.json" 2>/dev/null) in
ok) ;;
soon) stop skipped "the pool would refuse $pool_user within an hour" ;;
*) stop skipped "the pool listed no user $pool_user" ;;
esac

jq -cnR --argjson hide "$(jq -c '.notes + [.tracker]' <<<"$rotation")" --argjson caps '{"prompts": 60, "result": 2000, "budget": 700000}' \
	-f "$here/tail.jq" "$(field transcript)" >"$dir/tail.json" || stop failed "tail: tail.jq exited $?"
jq -j .text "$dir/tail.json" >"$dir/tail.txt"
[[ -s $dir/tail.txt ]] || stop skipped 'the cleared context left nothing to ask about'

fill extract lane="$lane" count="$count" tail@"$dir/tail.txt" >"$dir/extract.prompt.txt"
ask extract "$opus" "$dir/extract.prompt.txt" || stop failed "extract: $(why extract)"
jq 'if type == "array" then to_entries | map(.value + {q: (.key + 1)}) else error("the reply holds no list of questions") end' \
	"$dir/extract.reply.json" >"$dir/questions.json" 2>"$dir/questions.why" || stop failed "extract: $(why questions)"
asked=$(jq length "$dir/questions.json")
((asked >= 1 && asked <= count)) || stop failed "extract: $asked questions, not 1 to $count"
jq -r 'map("\(.q). \(.question)") | join("\n")' "$dir/questions.json" >"$dir/questions.txt"

fill answer lane="$lane" kit@"$dir/kit.txt" questions@"$dir/questions.txt" >"$dir/answer.prompt.txt"
ask "answer.$sonnet" "$sonnet" "$dir/answer.prompt.txt" &
answering=$!
ask "answer.$opus" "$opus" "$dir/answer.prompt.txt" || stop failed "answer by $opus: $(why "answer.$opus")"
wait "$answering" || stop failed "answer by $sonnet: $(why "answer.$sonnet")"

# The judge reads one model's answers as X and the other's as Y, by the low bit of the fresh context's id's SHA-256
byte=$(printf '%s' "$(field to)" | sha256sum | cut -c1-2)
if ((16#$byte & 1)); then x=$opus y=$sonnet; else x=$sonnet y=$opus; fi
jq -n --arg x "$x" --arg y "$y" '{X: $x, Y: $y}' >"$dir/key.json"
shown() { jq -r --slurpfile answers "$dir/answer.$1.reply.json" '($answers[0] | map({key: "\(.q)", value: .answer}) | from_entries) as $by
	| map("\(.q). \($by["\(.q)"] // "(no answer)")") | join("\n")' "$dir/questions.json"; }
shown "$x" >"$dir/x.txt"
shown "$y" >"$dir/y.txt"
jq -r 'map("\(.q). \(.question)\nReference: \(.answer)") | join("\n\n")' "$dir/questions.json" >"$dir/reference.txt"
fill judge reference@"$dir/reference.txt" x@"$dir/x.txt" y@"$dir/y.txt" >"$dir/judge.prompt.txt"
ask judge "$opus" "$dir/judge.prompt.txt" || stop failed "judge: $(why judge)"
jq -e --argjson grades "$grades" --argjson n "$asked" 'type == "array" and (map(.q) | sort) == [range(1; $n + 1)]
	and all(.[]; (.X | IN($grades[])) and (.Y | IN($grades[])))' "$dir/judge.reply.json" >/dev/null ||
	stop failed 'judge: not one grade of correct, partial, wrong or not stated for each answer'

row done "$(jq -nc --slurpfile judged "$dir/judge.reply.json" --slurpfile key "$dir/key.json" --argjson grades "$grades" --argjson n "$asked" \
	--arg sonnet "$sonnet" --arg opus "$opus" --argjson cost "$(jq -s 'map(.total_cost_usd) | add' "$dir"/{extract,"answer.$sonnet","answer.$opus",judge}.json)" '
	def counts($side): [$judged[0][][$side]] as $got | (reduce $grades[] as $g ({}; .[$g] = ($got | map(select(. == $g)) | length)))
		| . + {score: (.correct + .partial / 2)};
	($key[0] | to_entries | map({key: .value, value: .key}) | from_entries) as $sides
	| {questions: $n, sonnet: counts($sides[$sonnet]), opus: counts($sides[$opus]), $cost}')"
