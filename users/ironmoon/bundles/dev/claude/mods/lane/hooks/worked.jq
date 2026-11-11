# Where a session worked and the commits it made, newest first, from its transcript (`jq -cnR --arg home H -f worked.jq T`):
# `dirs`, the -R of its jj commands, the -C of its git ones, where it cd'd and the directories of files it edited or
# wrote, and after those the cwds its rows were written in, which for most sessions is where they started and not where
# they commit; absolute but not resolved. Only a command's lines up to its first heredoc are read, and only literal paths
# count, after cd only ones that look like a path, so a heredoc's body, prose, flags and variables give none. `ids`,
# whatever looks like a commit id in the output of its commands whose lines read so commit, push or land. Reads are left
# out: a file read names no workspace of the session's.
def text: if type == "string" then . else map(.text? // "") | join("\n") end;
def expanded: (if test("^'.*'$|^\".*\"$") then .[1:-1] else . end) | sub("^(~|\\$HOME|\\$\\{HOME\\})(?=/|$)"; $home);
def path($cwd): if startswith("/") then . else "\($cwd)/\(.)" end;
def lead: split("\n") | (map(test("<<")) | index(true)) as $heredoc | if $heredoc == null then . else .[:$heredoc + 1] end | join("\n");
def dirs($cwd):
  [lead | scan("(?:\\bjj\\b[^;&|\\n]*?\\s(?:-R|--repository)(?:\\s+|=)|\\bgit\\b[^;&|\\n]*?\\s-C\\s+)([^\\s;&|)]+)|(?:^|[\\s;&|(])cd\\s+([^\\s;&|)]+)")
   | .[0] // (.[1] | select(test("/|^\\.{1,2}$|^~")))
   | expanded | select(test("^-|[$`\\\\\"']") | not) | path($cwd)];
def distinct($n): reduce .[] as $x ({seen: {}, out: []}; if .seen[$x] or (.out | length) >= $n then . else .seen[$x] = true | .out += [$x] end) | .out;
def newest($n): reverse | distinct($n);
[inputs | fromjson? | select(.cwd != null) | .cwd as $cwd | (.message.content | arrays // []) as $content
  | {$cwd,
     dirs: [$content[] | select(.type == "tool_use") | .input as $in
       | if .name == "Bash" then $in.command // "" | dirs($cwd)[]
         elif .name | IN("Edit", "Write", "NotebookEdit") then $in.file_path // $in.notebook_path // empty | expanded | path($cwd) | sub("/[^/]*$"; "")
         else empty end],
     commands: [$content[] | select(.type == "tool_use" and .name == "Bash" and (.input.command // "" | lead | test("\\b(commit|push|land)\\b"))) | .id],
     results: [$content[] | select(.type == "tool_result") | {id: .tool_use_id, ids: [.content | text | scan("\\b[0-9a-f]{7,40}\\b")]}]}]
| (map(.commands[] | {(.): true}) | add // {}) as $committing
| {dirs: ((map(.dirs[]) | newest(32)) + (map(.cwd) | newest(32)) | distinct(32)), ids: ([.[].results[] | select($committing[.id]) | .ids[]] | newest(64))}
