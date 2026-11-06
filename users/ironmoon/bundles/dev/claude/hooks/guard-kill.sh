#!/usr/bin/env bash
# PreToolUse(Bash) guard: a session kills only processes it can show are its own. Pattern kills are
# refused, since other lanes' jobs match the same patterns: on 11-03 a lane's `pgrep -af ... ; kill`
# took another lane's 6-minute gate test (friction 96). A literal PID passes when its /proc cwd is
# inside the caller's cwd and that cwd is not a root other sessions share; a dead PID passes.
# `KILL_GUARD_OVERRIDE='<reason>' kill <pid>...` skips the ownership check for literal PIDs only
# and is logged (Owen 11-04 01:06: "change the guard so you can override with with an env var if needed").
# A pgrep whose pattern finds the shell running the line, on a line with a loop or a wait on a PID, and such a
# pidwait, are refused too: the line, and so the pattern, is in that shell's command line (friction 62).

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

[[ "$(jq -r '.tool_name // ""' <<<"$payload")" == Bash ]] || exit 0
cmd=$(jq -r '.tool_input.command // ""' <<<"$payload")
[[ ${cmd//[\\\'\"]/} =~ kill|fuser|pgrep|pidwait ]] || exit 0
cwd=$(realpath -m "$(jq -r '.cwd // empty' <<<"$payload")" 2>/dev/null) || cwd=/

own="Kill only PIDs you started: note \`\$!\` when you start the job and \`kill <that pid>\` from inside"
own+=" your own worktree, or stop your own background task with TaskStop."

block() {
  jq -cn --arg r "Blocked: $1" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
}
deny() { block "$1 $own"; }
by_pattern() { deny "\`$1\` picks processes by pattern, and other sessions' processes match it too."; }

mapfile -t shared < <(realpath -m / "$HOME" "$HOME/workspace/redox" "$HOME/workspace/claude" "$HOME"/research/* /etc/nixos "$HOME/tmp/worktree" /tmp)
from_bang=" $(grep -Eo '\b[A-Za-z_][A-Za-z0-9_]*=\$!' <<<"$cmd" | sed 's/=.*//' | tr '\n' ' ')"

check_kill() {
  local a i sig= dd= pids=() p t r
  local -a args=("$@")
  for ((i = 0; i < ${#args[@]}; i++)); do
    a=${args[i]}
    if [[ -z $dd ]]; then
      case $a in
        --) dd=1; continue ;;
        -l | -L | --list* | --table) return ;;
        -s | -n | --signal) ((i++)); sig=${args[i]:-}; continue ;;
        -*) sig=${a#-}; continue ;;
      esac
    fi
    case $a in
      +([0-9])) pids+=("$a") ;;
      -+([0-9])) mapfile -t -O "${#pids[@]}" pids < <(pgrep -g "${a#-}") ;;
      %* | '$!') ;;
      \$+([A-Za-z0-9_]) | \$\{+([A-Za-z0-9_])\})
        r=${a//[\$\{\}]/}
        [[ $from_bang == *" $r "* ]] || deny "\`kill $a\`: the hook can't see which process \$$r names." ;;
      '$') deny "\`kill \$(...)\` takes its PIDs from a command, and pgrep or pidof there match other sessions' processes too." ;;
      *) deny "\`kill $a\` is not a literal PID, so the hook can't see whose process it is." ;;
    esac
  done
  [[ $sig == 0 || $sig == SIG0 || $sig == NULL ]] && return
  [[ -n $override ]] && { overridden "${pids[@]}"; return; }
  for p in "${pids[@]}"; do
    [[ -e /proc/$p ]] || continue
    for r in "${shared[@]}"; do
      [[ $r == "$cwd" || $r == "$cwd"/* ]] &&
        deny "this session's cwd $cwd is shared with other sessions, so a PID's cwd can't show it is yours; cd into your own worktree first."
    done
    t=$(readlink "/proc/$p/cwd") || deny "PID $p's cwd is unreadable, so the hook can't see whose process it is."
    [[ $t == "$cwd" || $t == "$cwd"/* ]] ||
      deny "PID $p runs in $t, outside this session's cwd $cwd, so it belongs to another session."
  done
}

overridden() {
  local reason log=$HOME/.local/state/claude/kill-overrides.log p
  reason=$(grep -oE "KILL_GUARD_OVERRIDE=('[^']*'|\"[^\"]*\"|[^[:space:];&|]*)" <<<"$cmd" | head -1)
  reason=${reason#KILL_GUARD_OVERRIDE=}
  reason=${reason//[\'\"]/}
  [[ -n ${reason//[[:space:]]/} ]] || deny "KILL_GUARD_OVERRIDE needs a reason."
  mkdir -p "${log%/*}"
  {
    printf '%s\tsession %s\tcwd %s\treason %s\n' "$(date -Is)" "$(jq -r '.session_id // "-"' <<<"$payload")" "$cwd" "$reason"
    for p in "$@"; do printf '\tpid %s\t%s\n' "$p" "$(tr '\0' ' ' 2>/dev/null <"/proc/$p/cmdline" || echo '(gone)')"; done
  } >>"$log"
}

# The command line of the Bash tool's zsh running this line (read from /proc on 11-06), as pgrep -f reads it: newlines
# as blanks, other control characters as `?`.
zsh=$(realpath "$(command -v zsh 2>/dev/null)" 2>/dev/null)
wrapper=$(cat <<'EOF'
 2>/dev/null || true && : "${TMPDIR:=${CLAUDE_TMPDIR:-${CLAUDE_CODE_TMPDIR:-${TMP:-${TEMP:-/tmp}}}/claude-$(id -u)}}" export TMPDIR : && setopt NO_EXTENDED_GLOB NO_BARE_GLOB_QUAL 2>/dev/null || true && { \builtin unalias -- 'which'; \builtin unset -f -- 'which'; } >/dev/null 2>&1 || true && { \builtin unalias -- 'unsetenv'; \builtin unset -f -- 'unsetenv'; } >/dev/null 2>&1 || true && setopt NO_UNSET && eval
EOF
)
q=\''"'\''"'\'
as_pgrep_reads() { local l=${1//$'\n'/ }; printf '%s' "${l//[[:cntrl:]]/?}"; }
shell_line=$(as_pgrep_reads "${zsh:-zsh} -c source $HOME/.claude/shell-snapshots/snapshot-zsh-1793900368552-glc1vp.sh$wrapper '${cmd//\'/$q}' && pwd -P >| /tmp/claude-0000-cwd")

# What zsh's own lexer reads in line $1, each line of it alone too, and inside quoted scripts and substitutions, as
# NUL-separated records: `C`, a word count, then a pgrep or pidwait call's name and words, a word the shell expands
# (`$`, a backtick, a leading `~` or `=`, an unquoted glob) as $mask; `S` and the text of a quoted word that holds
# such a call, which an inner shell may run; `W` when a word loops or waits.
lexed() {
  timeout 5 zsh -fc '
    setopt extended_glob
    local -a todo=("$1") words args parts
    local -A seen
    local text w u q=$'"'"'\x27'"'"' d=\"
    local -i i j k
    while (( $#todo )); do
      text=$todo[1]
      shift todo
      [[ -n $seen[$text] ]] && continue
      seen[$text]=1
      [[ $text == *$'"'"'\n'"'"'* ]] && todo+=("${(@f)text}")
      words=(${(z)text})
      for ((i = 1; i <= $#words; i++)); do
        w=$words[i]
        [[ ${${(Q)w}:t} == (while|until|for|foreach|repeat|select|function|\(\)|wait-gone|pidwait|waitpid) || $w == --pi* ]] && print -rN W
        if [[ $w == *(pgrep|pidwait)* || ${(Q)w} == *(pgrep|pidwait)* ]]; then
          u=${(Q)w}
          [[ $u != $w ]] && { todo+=("$u"); print -rN S "$u"; }
          [[ $w == *\(* ]] && todo+=("${w#*\(}")
          parts=("${(@ps:\x60:)w}")
          for ((k = 2; k <= $#parts; k += 2)); do todo+=("$parts[k]"); done
        fi
        [[ ${${(Q)w}:t} == (pgrep|pidwait) ]] || continue
        args=()
        for ((j = i + 1; j <= $#words; j++)); do
          w=$words[j]
          [[ $w == (\;|\;\;|\||\|\||\&\&|\&|\|\&|\&\||\&\!|\)|\}) ]] && break
          if [[ $w == [0-9]#(\&|)[\<\>]##(\&|\||-|) ]]; then ((j++)); continue; fi
          u=${w//${q}[^${q}]#${q}/}
          [[ $u == *[\$\`]* || ${${u//${d}[^${d}]#${d}/}//\\?/} == ([~=]*|*[*?\[]*) ]] && args+=($2) || args+=("${(Q)w}")
        done
        print -rN C $#args ${${(Q)words[i]}:t} "${args[@]}"
      done
    done' zsh "$1" "$mask"
}

# The options tool $1 lists in its --help: `valued_of` the letters that take a value, `long_of` the long names, `=`
# after one that takes a value.
declare -A valued_of long_of
options_of() {
  [[ -v valued_of[$1] ]] && return
  local help
  help=$("$1" --help 2>/dev/null)
  valued_of[$1]=$(sed -nE 's/^ -([[:alpha:]]), --[a-z-]+ <.*/\1/p' <<<"$help" | tr -d '\n')
  long_of[$1]=$(sed -nE 's/^ +(-[[:alpha:]], )?--([a-z-]+)( <)?.*/\2\3/p' <<<"$help" | sed 's/ <$/=/' | tr '\n' ' ')
}
# The long option word $2 names for tool $1, as getopt reads an abbreviation; nothing when it names none or several.
long_option() {
  local o=${2#--} n m=()
  o=${o%%=*}
  for n in ${long_of[$1]}; do
    [[ ${n%=} == "$o" ]] && { echo "$n"; return; }
    [[ ${n%=} == "$o"* ]] && m+=("$n")
  done
  ((${#m[@]} == 1)) && echo "${m[0]}"
}

# Whether the call in `w` finds a shell running the line: its pattern, read as the regex it is (whole with -x), matches
# one's command line (with -f: the Bash tool's zsh, or an inner script) or name, or with no pattern its options select
# every process. `unread` is set when the hook can't read the pattern and takes it to match. -A doesn't help: another
# shell waiting on the same pattern still matches.
finds_itself() {
  local tool=${w[0]} a c o s pattern= has= options= full= whole= icase= invert= rc
  local -i i j
  local -a against
  unread=
  options_of "$tool"
  for ((i = 1; i < ${#w[@]}; i++)); do
    a=${w[i]}
    case $a in
      --) pattern=${w[i + 1]-} has=${w[i + 1]+1}; break ;;
      --?*)
        options=1
        o=$(long_option "$tool" "$a")
        case ${o%=} in
          full) full=1 ;;
          exact) whole=1 ;;
          ignore-case) icase=1 ;;
          inverse) invert=1 ;;
        esac
        [[ $o == *= && $a != *=* ]] && ((i++)) ;;
      -?*)
        options=1
        for ((j = 1; j < ${#a}; j++)); do
          c=${a:j:1}
          case $c in
            f) full=1 ;;
            x) whole=1 ;;
            i) icase=1 ;;
            v) invert=1 ;;
          esac
          [[ ${valued_of[$tool]} == *"$c"* ]] && { ((j + 1 < ${#a})) || ((i++)); break; }
        done ;;
      *) [[ -n $has ]] || pattern=$a has=1 ;;
    esac
  done
  [[ -n $has ]] || { [[ -n $options ]]; return; }
  [[ $pattern == *"$mask"* ]] && { unread=1; return 0; }
  [[ -n $whole ]] && pattern="^($pattern)\$"
  [[ -n $full ]] && against=("${subjects[@]}") || against=(zsh bash sh dash ksh)
  [[ -n $icase ]] && shopt -s nocasematch
  for s in "${against[@]}"; do
    [[ $s =~ $pattern ]] 2>/dev/null
    rc=$?
    # a pattern that isn't a regex makes pgrep fail at once
    ((rc == 2)) && break
    [[ -n $invert ]] && ((rc = !rc))
    ((rc == 0)) && break
  done
  shopt -u nocasematch
  ((rc == 0))
}
refuse() {
  local found
  [[ -n $unread ]] &&
    found="The hook can't read this \`${w[0]}\`'s pattern, which holds an expansion or a substitution, so it takes it to match the shell running the line" ||
    found="This \`${w[0]}\`'s pattern matches the shell running the line"
  [[ ${w[0]} == pidwait ]] && found+=", so it waits for that shell too and never returns." ||
    found+=", so it always finds that shell: waiting until it finds nothing never ends, and waiting until it finds something ends at once. The hook refuses such a pgrep anywhere on a line with a loop or a wait, since it doesn't tell which one the wait rests on."
  block "$found To wait for a job to end, note \`\$!\` when you start it and run \`~/workspace/claude/tools/wait-gone <pid>\` (\`--timeout <s>\` exits 124), or start it with run_in_background and take its notification. To look for a process by pattern, single-quote a pattern that doesn't match its own text, such as \`'[x]yz'\` for \`xyz\`, with \`xyz\` nowhere else on the line."
}

if [[ ${cmd//[\\\'\"]/} =~ pgrep|pidwait ]]; then
  mapfile -d '' out < <(lexed "$cmd"; echo -n "$?")
  [[ ${out[-1]} == 0 ]] || block "the hook couldn't read this line's pgrep calls, so it can't tell whether one waits on its own shell."
  subjects=("$shell_line") waits= calls=()
  for ((k = 0; k < ${#out[@]} - 1; k++)); do
    case ${out[k]} in
      W) waits=1 ;;
      S) subjects+=("$(as_pgrep_reads "${out[++k]}")") ;;
      C) calls+=("$k"); ((k += out[k + 1] + 2)) ;;
    esac
  done
  for k in "${calls[@]}"; do
    w=("${out[@]:k+2:out[k+1]+1}")
    finds_itself || continue
    [[ -n $waits || ${w[0]} == pidwait ]] && refuse
  done
fi

read_segments "$cmd"
while read -ra w; do
  lead=("${w[@]}")
  command_word 'kill|pkill|killall'
  ((${#w[@]})) || continue
  override=
  for a in "${lead[@]:0:${#lead[@]}-${#w[@]}}"; do [[ $a == KILL_GUARD_OVERRIDE=?* ]] && override=1; done
  case ${w[0]##*/} in
    pkill | killall | skill) by_pattern "${w[0]##*/}" ;;
    fuser) [[ " ${w[*]:1} " =~ \ -[[:alnum:]]*k ]] && deny "\`fuser -k\` kills every process using a file or port, whoever started it." ;;
    kill)
      [[ -n $fed ]] && by_pattern "xargs kill"
      check_kill "${w[@]:1}" ;;
  esac
done <<<"$lines"

exit 0
