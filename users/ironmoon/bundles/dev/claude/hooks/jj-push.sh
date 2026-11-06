#!/usr/bin/env bash
# `jj git push` given as jj's own arguments (`jj-push.sh [-R REPO] git push [OPTIONS]`), after the
# repo's pre-push hook has passed on what the push would send, fed to it as `git push` feeds it.
# guard-jj-push.sh refuses a bare `jj git push` wherever such a hook exists. The push runs on the
# dry run's snapshot of the working copy, so a hook run here checks the commits that get pushed.
#
# Nothing builds on desktop (Owen 11-05 21:53), so per clone `git config xb.prePushShell DIR` runs
# the hook as an xb job on the worktree's snapshot, in DIR's flake dev shell (empty: none), with
# each `xb.prePushEnv NAME` the caller's environment sets passed on. A job gets no stdin, so the ref
# lines go as arguments, and a hook outside the tracked tree is shipped beside it. The job's repo
# gets the pushed tips, the remote's refs for them, and each branch with an upstream, that upstream
# and the config naming it, so `branch@{upstream}` and a tip's history read as here.
# `xb.prePushLocal true` runs a hook that builds nothing here as before; a hooked clone with
# neither is refused.

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
hook=$(pre_push_hook "$root") || fail "$hook, so nothing was pushed"
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
setting() { env "${genv[@]}" git -C "$root" config --local "$@"; }
status=0
if [[ $(setting --bool --get xb.prePushLocal || true) == true ]]; then
  (cd "$root" && env "${genv[@]}" "$hook" "$remote" "$url" < <(printf '%s\n' "${refs[@]}")) || status=$?
  ((status == 0)) || fail "$hook exited $status, so nothing was pushed" "$status"
  exec jj "$@" "${own[@]}"
fi
shell=$(setting --get xb.prePushShell) ||
  fail "$hook would run here, where nothing may build: \`git config xb.prePushShell DIR\` runs it on xb, on the worktree's snapshot, in DIR's flake dev shell (empty: none); a hook that builds nothing may run here with \`git config xb.prePushLocal true\`"
xb=$HOME/.local/state/xb/current/bin/xb
[[ -x $xb ]] || fail "$hook runs on xb, and there is no $xb here"
real=$(realpath "$hook")
top=$(realpath "$root")
store=$(env "${genv[@]}" git -C "$root" rev-parse --path-format=absolute --git-common-dir)
snapshotted() {
  [[ $real != "$top"/.git/* && $real != "$top"/.jj/* ]] || return 1
  env "${genv[@]}" git -C "$root" check-ignore -q -- "$real"
  (($? == 1))
}
if snapshotted; then
  ship=() run=./${real#"$top"/}
else
  ! GIT_DIR=$store GIT_WORK_TREE=${real%/*} git check-ignore --no-index -q -- "$real" ||
    fail "$hook is ignored in its own directory, so xb would ship that directory without it; \`git config xb.prePushLocal true\` runs it here if it builds nothing"
  ship=(--with "${real%/*}=.xb-pre-push") run=./.xb-pre-push/${real##*/}
fi
in_shell=() pass=()
[[ -z $shell ]] || in_shell=(nix develop "$shell" -c)
while IFS= read -r name; do
  [[ $name =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || fail "xb.prePushEnv holds '$name', which is no variable's name"
  mapfile -d '' -t given < <(grep -azm1 "^$name=" /proc/$$/environ)
  pass+=("${given[@]}")
done < <(setting --get-all xb.prePushEnv || true)
declare -A named=()
state=()
ship_ref() { [[ -n ${named[$1]-} ]] || { named[$1]=1; state+=(--git-ref "$1${2:+=$2}"); }; }
exists() { env "${genv[@]}" git -C "$root" rev-parse -q --verify "$1" >/dev/null; }
for line in "${refs[@]}"; do
  read -r lref lsha rref _ <<<"$line"
  [[ $lsha == "$zero" ]] || ship_ref "$lref" "$lsha"
  tracking=refs/remotes/$remote/${rref#refs/heads/}
  ! exists "$tracking" || ship_ref "$tracking"
done
while read -r branch upstream; do
  [[ -z $upstream ]] || ! exists "$upstream" || { ship_ref "$branch"; ship_ref "$upstream"; }
done < <(env "${genv[@]}" git -C "$root" for-each-ref refs/heads \
  --format='%(if)%(symref)%(then)%(else)%(refname) %(upstream)%(end)')
while read -r key value; do
  state+=(--git-config "$key=$value")
done < <(setting --get-regexp '^(branch\..*\.(remote|merge)|remote\..*\.fetch)$' || true)
feed='remote=$1 url=$2; shift 2; printf "%s\n" "$@" | "$0" "$remote" "$url"'
(cd "$root" && "$xb" "${ship[@]}" "${state[@]}" env "${pass[@]}" "${in_shell[@]}" \
  bash -c "$feed" "$run" "$remote" "$url" "${refs[@]}") || status=$?
((status == 0)) || fail "$hook exited $status on xb (70 is also xb's own failure, in its words above), so nothing was pushed" "$status"
exec jj "$@" "${own[@]}"
