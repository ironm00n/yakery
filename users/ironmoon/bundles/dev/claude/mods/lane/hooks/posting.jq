# What a session sent to the tracker (`jq -rnR`): for each command that posts a comment or files an issue, one line
# "WHERE<TAB>TEXT" per place it sends to (posts.jq's targets: an issue's number, * for one a variable holds, "#" and
# the id of a comment edited, "new" for an issue filed), with TEXT as JSON. TEXT is the command itself and the content of every file it
# names that the session wrote before (by Write, or by a heredoc into a file), since a note is often posted from one.
# A file's content is kept to its first 64k characters, past which no text is matched whole.
include "posts" {search: "./"};
def heredocs: [scan("(?:cat|tee)\\s*(?:-a\\s+)?>?\\s*(\\S+)\\s*<<-?\\s*['\"]?(\\w+)['\"]?\\n([\\s\\S]*?)\\n\\2")] | map({(.[0] | gsub("[\"']"; "")): .[2][:65536]}) | add // {};
foreach (inputs | fromjson? | select(type == "object" and .type == "assistant" and (.isSidechain | not)) | .message.content[]? | select(.type == "tool_use")) as $call ({files: {}};
  .lines = []
  | if $call.name == "Write" then .files[$call.input.file_path // ""] = (($call.input.content // "")[:65536])
    elif $call.name == "Bash" then
      ($call.input.command // "") as $command
      | ($command | targets) as $targets
      | .lines = [[$command] + [.files | to_entries[] | select(.key as $path | $path != "" and ($command | contains($path) or contains($path | split("/") | last))) | .value]
          | .[] | tojson as $text | $targets[] | "\(.)\t\($text)"]
      | .files += ($command | heredocs)
    else . end;
  .lines[])
