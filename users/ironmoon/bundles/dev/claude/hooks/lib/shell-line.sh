# Command-line reading shared by the PreToolUse(Bash) guards. It reads the line, not a shell: an
# alias, a function, a script the line runs or a `bash -c` string is opaque to it.

shopt -s extglob
mask=$'\x1f'

# Quotes and comments resolved: a quoted word stays when it holds no blank or shell syntax and
# becomes $mask otherwise, while a `$(…)` or backtick inside double quotes stays code. A
# backslash-newline joins lines; any other escaped character becomes $mask.
unquote() {
  awk -v mask="$mask" '
    function word(lit, q) {
      return lit ~ /^[^[:space:];&|()<>`"\047\\]+$/ && !(q == "s" && lit ~ /\$/) ? lit : mask
    }
    function open_sub(c) {
      out = out c
      if (c == "`") mode[++top] = "b"; else { i++; mode[++top] = "c"; depth[top] = 0 }
    }
    { s = s (NR > 1 ? "\n" : "") $0 }
    END {
      top = 0; mode[0] = "n"; n = length(s)
      for (i = 1; i <= n; i++) {
        ch = substr(s, i, 1); two = substr(s, i, 2); m = mode[top]
        if (m == "s") { if (ch == "\047") { out = out word(lit, "s"); top-- } else lit = lit ch; continue }
        if (m == "d") {
          if (ch == "\"") { out = out (lit == "" && sub_in[top] ? "" : word(lit, "d")); top-- }
          else if (ch == "\\") lit = lit substr(s, ++i, 1)
          else if (ch == "`" || two == "$(") { out = out (lit == "" ? "" : mask); lit = ""; sub_in[top] = 1; open_sub(ch == "`" ? ch : two) }
          else lit = lit ch
          continue
        }
        if (ch == "\047" || ch == "\"") { mode[++top] = ch == "\"" ? "d" : "s"; lit = ""; sub_in[top] = 0 }
        else if (ch == "\\") { i++; if (substr(s, i, 1) != "\n") out = out mask }
        else if (ch == "#" && (i == 1 || substr(s, i - 1, 1) ~ /[[:space:];&|(]/)) { while (i < n && substr(s, i + 1, 1) != "\n") i++ }
        else if (m == "b" && ch == "`") { out = out ch; top-- }
        else if (ch == "`" || two == "$(") open_sub(ch == "`" ? ch : two)
        else if (m == "c" && ch == "(") { out = out ch; depth[top]++ }
        else if (m == "c" && ch == ")") { out = out ch; if (depth[top]-- == 0) top-- }
        else out = out ch
      }
      if (mode[top] == "s" || mode[top] == "d") out = out mask
      print out
    }'
}

# One simple command per line: heredoc bodies dropped; quotes and comments resolved by `unquote`;
# redirections dropped; `$(` and backticks left as a `$` argument; each paren on a line of its own,
# so a caller can scope a subshell's `cd`.
segments() {
  awk 'w != "" { if ($0 ~ "^\t*" w "$") w = ""; next }
       { print }
       match($0, /(^|[^<])<<-?[[:space:]]*["\047]?[A-Za-z_][A-Za-z0-9_]*/) {
         w = substr($0, RSTART, RLENGTH); sub(/^[^<]?<<-?[[:space:]]*["\047]?/, "", w) }' <<<"$1" |
    unquote |
    sed -E -e 's/[0-9]*[<>]+&?[[:space:]]*[^[:space:];&|()]*//g' -e 's/\$\(/$\n(/g' -e 's/`/$\n/g' \
      -e 's/[()]/\n&\n/g' -e 's/&&|\|\||[;&|]/\n/g'
}

# Drops the words before the command proper (sudo, env, timeout, VAR=x, if, ...) from the array `w`.
# After xargs, words up to the first one named by the extglob $1 go too, and `fed` is set; an
# `env -C` directory lands in `chdir`.
command_word() {
  fed= chdir=
  while ((${#w[@]})); do
    case ${w[0]} in
      sudo | command | builtin | exec | nohup | setsid | nice | time | env | if | elif | while | until | then | do | else | '!' | '{') w=("${w[@]:1}") ;;
      -C | --chdir) chdir=${w[1]-}; w=("${w[@]:2}") ;;
      --chdir=*) chdir=${w[0]#*=}; w=("${w[@]:1}") ;;
      -* | *=*) w=("${w[@]:1}") ;;
      timeout) w=("${w[@]:2}") ;;
      xargs) fed=1; w=("${w[@]:1}") ;;
      *) [[ -n $fed && ${w[0]##*/} != @($1) ]] && w=("${w[@]:1}") || break ;;
    esac
  done
}
