# Where commit messages break the width rule, from git log's NUL-separated `<sha>\n<message>` records: a message's
# first line and each body line fit in $width characters, except a body line that can't break (one that is little
# more than a URL, code span or word too long for any line), a line in a fenced block, and the closing trailers.
# Emits `<sha> {subject, lines, passed}` for each that doesn't: the first line's length where it is over, the numbers
# of the body lines over, and how many lines over passed as exceptions.

# All but a short lead-in of the line is units no line could hold.
def unbreakable:
  ($width - (match("^\\s*").length)) as $room
  | ([match("`[^`]*`|[^\\s`]+"; "g").string | select(length > $room)] | map(length) | add // 0) as $long
  | $long > 0 and length - $long <= 20;

def problems:
  split("\n") | map(rtrim) | until(length == 0 or last != ""; .[:-1]) as $l
  | ([range($l | length) | select($l[.] | test("\\S"))] | first) as $s
  | select($s != null)
  | ([range($l | length) | select($l[.] == "")] | last // -1) as $blank
  | ($l[$blank + 1:]) as $tail
  | (if $blank > $s and ($tail[0] | test("^[A-Za-z][A-Za-z0-9-]*: ")) and ($tail | all(test("^[A-Za-z][A-Za-z0-9-]*: |^\\s+\\S")))
     then $blank + 1
     else $l | length end) as $trailers
  | [foreach range($l | length) as $i ({in: false};
      ($i > $s and ($l[$i] | test("^\\s*```"))) as $f | {in: (if $f then .in | not else .in end), fence: $f};
      .fence or .in)] as $fenced
  | [range($s + 1; $l | length) | select(($l[.] | length) > $width)
     | {line: (. + 1), exempt: (. >= $trailers or $fenced[.] or ($l[.] | unbreakable))}] as $long
  | ($l[$s] | length) as $first
  | {subject: (if $first > $width then $first else null end), lines: [$long[] | select(.exempt | not) | .line],
     passed: ([$long[] | select(.exempt)] | length)}
  | select(.subject != null or (.lines | length) > 0);

split("\u0000")[] | select(. != "") | index("\n") as $i | .[:$i] as $sha | .[$i + 1:] | problems | "\($sha) \(tojson)"
