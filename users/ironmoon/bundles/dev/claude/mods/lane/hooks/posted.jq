# The ids of the issue comments, and the numbers of the issues, a session's own commands posted or edited (`jq -cnR`),
# as {comments, issues}, so kit B can tell a lane's own from everyone else's: all come from the one agent account, so
# only the transcript knows whose they are. Each is what the command's output printed: the comment's URL
# (…#issuecomment-N) or its JSON (the first "id"); the issue's URL (…/issues/N) or its JSON ("number"). A number alone on
# a line is neither, since on 11-10 most such were issue numbers or HTTP statuses.
include "posts" {search: "./"};
def comment_ids:
  [scan("issuecomment-([0-9]+)")[0] | tonumber] as $urls
  | if $urls != [] then $urls else [capture("\"id\"\\s*:\\s*(?<id>[0-9]+)").id | tonumber] end;
def issue_numbers:
  [scan("/issues/([0-9]+)(?![0-9]|#)")[0] | tonumber] as $urls
  | if $urls != [] then $urls else [capture("\"number\"\\s*:\\s*(?<n>[0-9]+)").n | tonumber] end;
[foreach (inputs | fromjson? | select(type == "object" and (.isSidechain | not))) as $row ({posting: {}, new: []};
  . as $state
  | if $row.type == "assistant" then
      .new = []
      | .posting += ([$row.message.content[]? | select(.type == "tool_use" and .name == "Bash") | .id as $id | .input.command
          | [if posts then "comments" else empty end, if files then "issues" else empty end] | select(. != []) | {($id): .}] | add // {})
    elif $row.type == "user" and ($row.message.content | type) == "array" then
      .new = [$row.message.content[] | select(.type? == "tool_result") | ($state.posting[.tool_use_id] // []) as $kinds
        | .content // "" | text | (if "comments" | IN($kinds[]) then comment_ids[] | {comments: .} else empty end), (if "issues" | IN($kinds[]) then issue_numbers[] | {issues: .} else empty end)]
    else .new = [] end;
  .new[])]
| {comments: (map(.comments // empty) | unique), issues: (map(.issues // empty) | unique)}
