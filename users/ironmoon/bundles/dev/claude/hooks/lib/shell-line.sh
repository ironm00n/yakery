# Command-line reading shared by the PreToolUse(Bash) guards. It reads the line, not a shell: an
# alias, a function or a script the line runs is opaque to it.

shopt -s extglob
mask=$'\x1f'

# Quotes and comments resolved: a quoted word stays when it holds no blank or shell syntax and
# becomes $mask otherwise, while a `$(…)` or backtick inside double quotes stays code. The quoted
# script of `sh -c` (bash, zsh, ...) becomes a parenthesized subshell, left quoted for the next
# pass. A backslash-newline joins lines; any other escaped character becomes $mask.
unquote() {
  awk -v mask="$mask" '
    BEGIN { shell_c = "(^|[[:space:];&|(`])([^[:space:];&|()]*/)?(ba|da|z|k)?sh[[:space:]]+(([-+][[:alnum:]-]+|-o[[:space:]]+[[:alnum:]]+)[[:space:]]+)*-[[:alnum:]]*c[[:alnum:]]*[[:space:]]+$" }
    function word(lit, q) {
      return lit ~ /^[^#[:space:];&|()<>`"\047\\][^[:space:];&|()<>`"\047\\]*$/ && !(q == "s" && lit ~ /\$/) ? lit : mask
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
        if (m == "s") { if (ch == "\047") { out = out (script[top] ? "(" lit ")" : word(lit, "s")); top-- } else lit = lit ch; continue }
        if (m == "d") {
          if (ch == "\"") { out = out (script[top] ? "(" lit ")" : lit == "" && sub_in[top] ? "" : word(lit, "d")); top-- }
          else if (ch == "\\") lit = lit substr(s, ++i, 1)
          else if (!script[top] && (ch == "`" || two == "$(")) { out = out (lit == "" ? "" : mask); lit = ""; sub_in[top] = 1; open_sub(ch == "`" ? ch : two) }
          else lit = lit ch
          continue
        }
        if (ch == "\047" || ch == "\"") { mode[++top] = ch == "\"" ? "d" : "s"; lit = ""; sub_in[top] = 0; script[top] = out ~ shell_c }
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

# One simple command per line: heredoc bodies dropped; quotes and comments resolved by `unquote`,
# until no `sh -c` script is left quoted; redirections dropped; `$(` and backticks left as a `$`
# argument; each paren on a line of its own, so a caller can scope a subshell's `cd`. Fails when a step does.
segments() {
  local line prev
  line=$(awk 'w != "" { if ($0 ~ "^\t*" w "$") w = ""; next }
       { print }
       match($0, /(^|[^<])<<-?[[:space:]]*["\047]?[A-Za-z_][A-Za-z0-9_]*/) {
         w = substr($0, RSTART, RLENGTH); sub(/^[^<]?<<-?[[:space:]]*["\047]?/, "", w) }' <<<"$1") || return
  until [[ $line == "${prev-}" ]]; do
    prev=$line
    line=$(unquote <<<"$line") || return
  done
  sed -E -e 's/[0-9]*[<>]+&?[[:space:]]*[^[:space:];&|()]*//g' -e 's/\$\(/$\n(/g' -e 's/`/$\n/g' \
    -e 's/[()]/\n&\n/g' -e 's/&&|\|\||[;&|]/\n/g' <<<"$line"
}

# Sets `lines` to line $1's `segments`. A line they fail on blocks the call, by a PreToolUse hook's exit 2, so a fault
# in the reader can't let every command through.
read_segments() {
  lines=$(segments "$1") && return
  echo "Blocked: the hook couldn't read this command line, so it can't tell what the line runs." >&2
  exit 2
}

# The absolute path a word names, from base dir $2 (empty when unknown); nothing if it can't be read
# off the line.
dir_of() {
  local p=$1
  [[ $p == '~' || $p == '~/'* ]] && p=$HOME${p:1}
  p=${p//'${HOME}'/$HOME}
  p=${p//'$HOME'/$HOME}
  [[ -z $p || $p == *[\$\`*?[$mask]* || ($p != /* && -z $2) ]] && return
  [[ $p == /* ]] || p=$2/$p
  realpath -m -- "$p"
}

# Calls $1 on each simple command of line $2 run from dir $3, with `w`, `fed` and `chdir` set by
# `command_word "$4"` and `here` the dir it runs in, empty when the line doesn't say (a variable,
# `cd -`). `cd` and `pushd` move it, a subshell scopes it.
each_command() {
  local f=$1 cwd=$3 names=$4 scopes=() to x lines
  read_segments "$2"
  while read -ra w; do
    case ${w[0]-} in
      '(') scopes+=("$cwd"); continue ;;
      ')') ((${#scopes[@]})) && { cwd=${scopes[-1]}; unset 'scopes[-1]'; }; continue ;;
    esac
    command_word "$names"
    ((${#w[@]})) || continue
    here=$cwd
    [[ -n $chdir ]] && here=$(dir_of "$chdir" "$cwd")
    case ${w[0]##*/} in
      cd | pushd)
        to='~'
        for x in "${w[@]:1}"; do [[ $x == @(-[LPe@]*|--) ]] || { to=$x; break; }; done
        [[ $to == - ]] && cwd= || cwd=$(dir_of "$to" "$cwd") ;;
      *) "$f" ;;
    esac
  done <<<"$lines"
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
