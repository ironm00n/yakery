#!/usr/bin/env bash
# `jj git push` given as jj's own arguments (`jj-push.sh [-R REPO] git push [OPTIONS]`), after the
# repo's pre-push hook has passed on what the push would send, fed to it as `git push` feeds it.
# guard-jj-push.sh refuses a bare `jj git push` wherever such a hook exists. The push runs on the
# dry run's snapshot of the working copy, so the hook checks the commits that get pushed.

set -euo pipefail
. "${BASH_SOURCE[0]%/*}/lib/jj.sh"

repo=() own=(--ignore-working-copy)
for ((i = 1; i <= $#; i++)); do
  case ${!i} in
    --ignore-working-copy) own=() ;;
    -R | --repository) ((i++)); repo=(-R "${!i}") ;;
    --repository=*) repo=(-R "${!i#*=}") ;;
    -R?*) repo=(-R "${!i#-R}") ;;
  esac
done
fail() { echo "jj-push: $1" >&2; exit "${2:-1}"; }
jjr() { jj "${repo[@]}" --ignore-working-copy "$@"; }

root=$(jjr workspace root)
hook=$(pre_push_hook "$root")
[[ -n $hook ]] || exec jj "$@"

plan=$(jj "$@" --dry-run 2>&1) || fail "the dry run failed: $plan" $?
zero=0000000000000000000000000000000000000000
full() { jjr log --no-graph -r "$1" -T commit_id; }
remote= refs=() warning=0
bookmark='^  bookmark: ([^ ]+) \[(add to ([0-9a-f]+)|move (forward|backward|sideways) from ([0-9a-f]+) to ([0-9a-f]+)|delete from ([0-9a-f]+))\]$'
while IFS= read -r line; do
  # a warning (`Won't push bookmark b: commit c is private`) and the lines indented under it are jj's, not the plan's
  [[ $line == '  '* ]] || warning=0
  if [[ $line == Warning:* ]]; then
    warning=1
  elif ((warning)); then
    :
  elif [[ $line =~ ^Changes\ to\ push\ to\ (.+):$ ]]; then
    remote=${BASH_REMATCH[1]}
  elif [[ $line =~ $bookmark ]]; then
    m=("${BASH_REMATCH[@]}")
    ref=refs/heads/${m[1]}
    if [[ -n ${m[3]} ]]; then refs+=("$ref $(full "${m[3]}") $ref $zero")
    elif [[ -n ${m[7]} ]]; then refs+=("(delete) $zero $ref $(full "${m[7]}")")
    else refs+=("$ref $(full "${m[6]}") $ref $(full "${m[5]}")")
    fi
  elif [[ $line == '  '* ]]; then
    fail "can't read this line of jj's plan, so the hook can't be fed it: $line"
  fi
done <<<"$plan"
((${#refs[@]})) || exec jj "$@" "${own[@]}"
[[ -n $remote ]] || fail "jj's plan names no remote: $plan"
url=$(jjr git remote list | awk -v r="$remote" '$1 == r { print $2 }')

git_env "$root"
status=0
(cd "$root" && env "${genv[@]}" "$hook" "$remote" "$url" < <(printf '%s\n' "${refs[@]}")) || status=$?
((status == 0)) || fail "$hook exited $status, so nothing was pushed" "$status"
exec jj "$@" "${own[@]}"
