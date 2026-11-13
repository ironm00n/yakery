#!/usr/bin/env bash
# `jj git push` given as jj's own arguments (`jj-push.sh [-R REPO] git push [OPTIONS]`), after the
# commits it would send that aren't on the remote yet have passed the commit width rule (lib/widths.sh)
# and the provenance rule (lib/provenance.sh), and the repo's pre-push hook, if it has one, has passed on what the push would send, fed to it as
# `git push` feeds it. guard-jj-push.sh refuses a bare `jj git push` in a Claude Code session. The
# push runs on the dry run's snapshot of the working copy, and the agent's commits in it are signed
# before the checks, so what is checked here is what gets pushed.
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
. "${BASH_SOURCE[0]%/*}/lib/widths.sh"
. "${BASH_SOURCE[0]%/*}/lib/provenance.sh"

repo=() own=(--ignore-working-copy) conf=()
for ((i = 1; i <= $#; i++)); do
  case ${!i} in
    --ignore-working-copy) own=() ;;
    --config | --config-file) conf+=("${!i}") && ((i++)) && conf+=("${!i}") ;;
    --config=* | --config-file=*) conf+=("${!i}") ;;
    -R | --repository) ((i++)); repo=(-R "${!i}") ;;
    --repository=*) repo=(-R "${!i#*=}") ;;
    -R?*) repo=(-R "${!i#-R}") ;;
  esac
done
fail() { echo "jj-push: $1" >&2; exit "${2:-1}"; }
jjr() { jj "${repo[@]}" --ignore-working-copy "$@"; }

root=$(jjr workspace root)
hook=$(pre_push_hook "$root") || fail "$hook, so nothing was pushed"

# the plan is read off the dry run's output, so a dry run made quiet by `--quiet` would read as nothing to push; one
# quiet by config fails below, as an empty plan that doesn't say nothing changed (a second --config would drop the
# caller's, as jj 0.46 reads them)
loud=()
for a in "$@"; do [[ $a == --quiet ]] || loud+=("$a"); done
# what the push sends, read off a dry run into `refs` (pre-push lines) and `sending` (the remotes)
read_plan() {
  plan=$(jj "${loud[@]}" --dry-run 2>&1) || fail "the dry run failed: $plan" $?
  sending=() remote= refs=() warning=0
  zero=0000000000000000000000000000000000000000
  git_env "$root"
  full() { jjr log --no-graph -r "$1" -T commit_id; }
  # jj's plan quotes a bookmark, tag or remote name that isn't a plain revset symbol (`"pkg@1.2.3"`), escaping a `"`
  unquoted() {
    local name=$1
    [[ $name == \"*\" ]] || { printf '%s\n' "$name"; return; }
    name=${name:1:-1}
    printf '%s\n' "${name//\\\"/\"}"
  }
  planned='^  (bookmark|tag): ([^ ]+) \[(add to ([0-9a-f]+)|move (forward|backward|sideways) from ([0-9a-f]+) to ([0-9a-f]+)|delete from ([0-9a-f]+))\]$'
  while IFS= read -r line; do
    # a warning (`Won't push bookmark b: commit c is private`) and the lines indented under it are jj's, not the plan's
    [[ $line == '  '* ]] || warning=0
    if [[ $line == Warning:* ]]; then
      warning=1
    elif ((warning)); then
      :
    elif [[ $line =~ ^Changes\ to\ push\ to\ (.+):$ ]]; then
      remote=$(unquoted "${BASH_REMATCH[1]}")
    elif [[ $line =~ $planned ]]; then
      m=("${BASH_REMATCH[@]:1}")
      [[ -n $remote ]] || fail "jj's plan names no remote before this line: $line"
      sending[$remote]=1
      name=$(unquoted "${m[1]}")
      [[ ${m[0]} == bookmark ]] && ref=refs/heads/$name || ref=refs/tags/$name
      if [[ -n ${m[3]} ]]; then refs+=("$ref $(full "${m[3]}") $ref $zero")
      elif [[ -n ${m[7]} ]]; then refs+=("(delete) $zero $ref $(full "${m[7]}")")
      else refs+=("$ref $(full "${m[6]}") $ref $(full "${m[5]}")")
      fi
    elif [[ $line == '  '* ]]; then
      fail "can't read this line of jj's plan, so the hook can't be fed it: $line"
    fi
  done <<<"$plan"
}
declare -A sending=()
read_plan
if ((${#refs[@]} == 0)); then
  [[ $plan == *$'\n'"Nothing changed."* || $plan == "Nothing changed."* ]] ||
    fail "jj's plan names nothing to push, and doesn't say nothing changed, so it can't be read: $plan"
  exec jj "$@" "${own[@]}"
fi
problems=$(printf '%s\n' "${refs[@]}" | (((${#genv[@]})) && export "${genv[@]}"; cd "$root" && too_wide)) ||
  fail "couldn't read the commits this push sends, so nothing was pushed"
change() { jjr log --no-graph -r "$1" -T 'change_id.shortest(8)'; }
[[ -z $problems ]] ||
  fail "$(too_wide_reason "$problems" change "Nothing was pushed. Fix each with \`jj describe -r <change> -m '…'\`, then push again.")"
unmarked=$(printf '%s\n' "${refs[@]}" | (((${#genv[@]})) && export "${genv[@]}"; cd "$root" && unmarked)) ||
  fail "couldn't read the agent identity or the commits this push sends, so nothing was pushed"
[[ -z $unmarked ]] ||
  fail "$(unmarked_reason "$unmarked" change "Nothing was pushed. Add the trailer with \`jj describe -r <change>\`, then push again.")"
# the agent's commits this push sends are signed here, by the key jj is set to, so the check below reads what the push
# sends (jj would otherwise sign them only as it pushes); messages are checked above, as signing leaves them be
in_repo() { printf '%s\n' "${refs[@]}" | (((${#genv[@]})) && export "${genv[@]}"; cd "$root" && "$@"); }
agents=$(in_repo agent_commits) || fail "couldn't read the agent identity or the commits this push sends, so nothing was pushed"
if [[ -n $agents ]]; then
  signed=$(jj "${repo[@]}" "${conf[@]}" sign -r "$(paste -sd'|' <<<"$agents")" 2>&1) ||
    fail "jj couldn't sign the agent's commits this push sends, so nothing was pushed: $signed"
  read_plan
fi
unsigned=$(in_repo unsigned) ||
  fail "couldn't read the agent identity or the commits this push sends, so nothing was pushed"
[[ -z $unsigned ]] ||
  fail "$(unsigned_reason "$unsigned" change "Nothing was pushed. jj here signs them with another key: set signing.key to the agent's, as the claude bundle's jj scope does, then push again.")"
rewritten=$(printf '%s\n' "${refs[@]}" | (((${#genv[@]})) && export "${genv[@]}"; cd "$root" && rewritten_main))
[[ -z $rewritten ]] ||
  fail "this push rewrites or deletes $(cut -d' ' -f1 <<<"$rewritten" | paste -sd' '), which only moves forward; nothing was pushed. A feature branch of ours is rewritten and force-pushed; main and master never"
[[ -n $hook ]] || exec jj "$@" "${own[@]}"
((${#sending[@]} == 1)) ||
  fail "this push sends to ${!sending[*]}, and git runs the hook once per remote with that remote's lines; push to one at a time"
remote=${!sending[*]}
# git feeds an annotated tag by its tag object, where jj's plan names the commit it points at; a tag git's store
# doesn't hold is one jj made, and goes as the commit
for i in "${!refs[@]}"; do
  read -r lref lsha rref rsha <<<"${refs[i]}"
  [[ $lref == refs/tags/* ]] || continue
  object=$(env "${genv[@]}" git -C "$root" rev-parse -q --verify "$lref") || continue
  [[ $(env "${genv[@]}" git -C "$root" rev-parse "$object^{commit}") == "$lsha" ]] ||
    fail "git's $lref doesn't point at $lsha as jj's plan says, so the hook can't be fed it"
  refs[i]="$lref $object $rref $rsha"
done
urls=$(env "${genv[@]}" git -C "$root" remote get-url --push --all -- "$remote") ||
  fail "git has no push URL for $remote, so the hook can't be given one"
[[ $urls != *$'\n'* ]] ||
  fail "$remote pushes to $(wc -l <<<"$urls") URLs, and git runs the hook once per URL; push with one"
url=$urls
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
