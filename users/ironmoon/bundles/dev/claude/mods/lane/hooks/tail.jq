# The recall check's view of a cleared context (`jq -cnR --argjson hide '[text, ...]' --argjson caps '{"prompts": n,
# "result": n, "budget": n}'`), the replay's extractor view (exp/scripts/replay/tail.py): its last $caps.prompts prompts
# from Owen or another session and what lies between them, in file order, to its end. What either arm keeps notes in is
# not shown: the result of a tool call whose input names any of $hide (the state file, the tracker) is left out, as the
# replay left out the handoff file. Nor are the rows the lane mod put in (its kits, asks and its prompt to carry on): they
# are what the check tests. Assistant text is kept whole, a tool result or report
# when at most $caps.result characters. Past $caps.budget characters it takes the first level that fits: without tool
# results, then also without reports, then the newest $caps.budget characters of prompts and assistant text.
def text: if type == "string" then . else map(if .type == "text" then .text else "[\(.type)]" end) | join("\n") end;
def said($origin; $content):
  if $origin.kind == "human" then {who: "OWEN", text: ($content | text)}
  elif $origin.kind == "peer" and ($origin.handback | not) and $origin.senderTaskId == null then
    {who: "PEER \($origin.name // $origin.from)", text: ($origin.body // ($content | text))}
  else empty end;
def kept($label): if length <= $caps.result then . else "[\($label) of \(length) characters left out]" end;
def event($kind; $at; $line): {kind: $kind, line: "[\($at)Z] \($line)"};
def names_hidden: tojson as $input | any($hide[]; . as $text | $input | contains($text));
def events($hidden):
  ((.timestamp // "")[:16] | sub("T"; " ")) as $at
  | (.origin.kind // "other" | ascii_upcase) as $kind
  | if .type == "attachment" then
      .attachment | select(.type == "queued_command") | said(.origin // {}; .prompt // "") | event("prompt"; $at; "\(.who):\n\(.text)")
    elif .type == "assistant" then
      .message.content[]? | select(.type == "text" and (.text | test("\\S"))) | event("text"; $at; "ASSISTANT:\n\(.text)")
    elif .type == "user" then
      .message.content as $content
      | if ($content | type) == "array" and any($content[]; .type? == "tool_result") then
          $content[] | select(.type? == "tool_result" and ($hidden[.tool_use_id] | not)) | event("result"; $at; "TOOL RESULT: \(.content // "" | text | kept("result"))")
        elif .isMeta then empty
        else (said(.origin // {}; $content) | event("prompt"; $at; "\(.who):\n\(.text)"))
          // ($content | text | select(test("\\S")) | event("report"; $at; "\($kind) REPORT: \(kept("report"))"))
        end
    else empty end;
[foreach (inputs | fromjson? | select(type == "object")) as $row ({};
    . + ([$row | select(.type == "assistant") | .message.content[]? | select(.type == "tool_use" and (.input | names_hidden)) | {(.id): true}] | add // {});
    . as $hidden | $row | select((.isSidechain | not) and (.isCompactSummary | not) and .origin.name != "lane") | events($hidden))] as $seen
| [$seen | to_entries[] | select(.value.kind == "prompt") | .key] as $starts
| $seen[(if ($starts | length) >= $caps.prompts then $starts[-$caps.prompts] else 0 end):] as $window
| [["prompt", "text", "result", "report"], ["prompt", "text", "report"], ["prompt", "text"]]
| map(. as $kinds | [$window[] | select(.kind | IN($kinds[])) | .line] | join("\n\n")) as $levels
| ([range($levels | length) | select($levels[.] | length <= $caps.budget)] | first // ($levels | length)) as $level
| {prompts: ([$window[] | select(.kind == "prompt")] | length), $level,
   text: (if $level < ($levels | length) then $levels[$level] else $levels[-1][-$caps.budget:] end)}
