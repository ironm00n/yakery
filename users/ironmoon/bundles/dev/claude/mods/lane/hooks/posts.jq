# What a session's Bash command does to the tracker's comments, for posted.jq and posting.jq (`include "posts";`).
def text: if type == "string" then . else map(if .type == "text" then .text else "" end) | join("\n") end;
# it sends data: a curl flag, or an HTTP call made with "POST" or "PATCH" (lanes also post from Python's urllib)
def sends: test("-X *(POST|PATCH)|--request +(POST|PATCH)|(^|\\s)(-d|-F|--data[a-z-]*|--json|--form)(\\s|=|$)|[\"\\x27](POST|PATCH)[\"\\x27]");
def patches: test("PATCH") and test("comments/[0-9]+");
# it adds a comment when it sends data to a path ending in an issue's comments, however the URL is built
# ($api/$n/comments, "%d/comments" % n), or edits one by its URL (comments/<id>)
def posts: sends and test("/comments\\b");
# it files an issue when it sends data to a path ending in /issues
def files: sends and test("/issues(?![/0-9A-Za-z_])");
# where its text goes: each issue it posts a comment to (by number, or * when a variable holds it), "#" and the id of
# each comment it edits, and "new" when it files an issue; one command can do all three
def targets:
  [(if patches then scan("comments/([0-9]+)")[0] | "#\(.)" else empty end),
   (if posts then scan("([0-9]+)/comments\\b")[0], (if test("(?<![0-9])(?<!/issues)/comments\\b") then "*" else empty end) else empty end),
   (if files then "new" else empty end)] | unique;
