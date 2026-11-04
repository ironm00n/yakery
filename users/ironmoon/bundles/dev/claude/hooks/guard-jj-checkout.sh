#!/usr/bin/env bash
# PreToolUse(Bash) guard: jj snapshots its working copy before every command, even one that then
# fails, so a jj run in one of Owen's own checkouts writes whatever sits on disk there into his @.
# On 11-04 a subagent's cwd reset to ~/workspace/redox and a `jj file show -r <another workspace's
# change>` amended his @ with 65 files (friction 98). A jj whose repo is one of his checkouts passes
# only with --ignore-working-copy. The one exception is the foreground main session started in
# ~/workspace/claude (dispatch), on that checkout, for its rebases onto master.

set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/shell-line.sh"

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // \"\"" <<<"$payload"; }
[[ $(field .tool_name) == Bash ]] || exit 0
cmd=$(field .tool_input.command)
[[ $cmd == *jj* ]] || exit 0

mapfile -t checkouts < <(realpath -m "$HOME/workspace/redox" "$HOME/workspace/claude" /etc/nixos "$HOME"/research/*)
dispatch=
[[ -z $(field .agent_id) && -z ${CLAUDE_JOB_DIR-} &&
  $(field .transcript_path) == "$HOME/.claude/projects/${HOME//[^[:alnum:]]/-}-workspace-claude/"* ]] &&
  dispatch=$(realpath -m "$HOME/workspace/claude")

instead="Run it against your own jj workspace with \`-R <your workspace>\`, or add \`--ignore-working-copy\` for a read."
deny() {
  jq -cn --arg r "Blocked: $1 $instead" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  exit 0
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

repo_of() {
  local d=$1
  until [[ -d $d/.jj ]]; do
    [[ $d == / ]] && return 1
    d=${d%/*}
    d=${d:-/}
  done
  echo "$d"
}

check_jj() {
  local a i r=$here repo c
  for ((i = 1; i <= $#; i++)); do
    a=${!i}
    case $a in
      --) break ;;
      --ignore-working-copy) return ;;
      -R | --repository) ((i++)); r=$(dir_of "${!i-}" "$here") ;;
      --repository=*) r=$(dir_of "${a#*=}" "$here") ;;
      -R?*) r=$(dir_of "${a#-R}" "$here") ;;
    esac
  done
  [[ -n $r ]] || deny "the hook can't read off the line which repo this \`jj\` runs in (a variable, substitution, quoted path with blanks or \`cd -\`), so it may be one of Owen's checkouts."
  repo=$(repo_of "$r") || return
  for c in "${checkouts[@]}"; do
    [[ $repo == "$c" && $repo != "$dispatch" ]] &&
      deny "this \`jj\` runs in $repo, Owen's own checkout, and jj snapshots the files on disk into his @ before any command, even one that then fails."
  done
}

cwd=$(dir_of "$(field .cwd)" "")
scopes=()
while read -ra w; do
  case ${w[0]-} in
    '(') scopes+=("$cwd"); continue ;;
    ')') ((${#scopes[@]})) && { cwd=${scopes[-1]}; unset 'scopes[-1]'; }; continue ;;
  esac
  command_word jj
  ((${#w[@]})) || continue
  here=$cwd
  [[ -n $chdir ]] && here=$(dir_of "$chdir" "$cwd")
  case ${w[0]##*/} in
    cd | pushd)
      to='~'
      for x in "${w[@]:1}"; do [[ $x == @(-[LPe@]*|--) ]] || { to=$x; break; }; done
      [[ $to == - ]] && cwd= || cwd=$(dir_of "$to" "$cwd") ;;
    jj) check_jj "${w[@]:1}" ;;
  esac
done < <(segments "$cmd")

exit 0
