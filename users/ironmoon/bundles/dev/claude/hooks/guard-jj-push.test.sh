h="${1:-$HOME/.claude/hooks/guard-jj-push.sh}"
p="${2:-${h%/*}/jj-push.sh}"
dir=$(realpath "$(mktemp -d)")
trap 'rm -rf "${dir:?}"' EXIT
export JJ_CONFIG="$dir/jj.toml" GIT_CONFIG_GLOBAL=/dev/null
printf '[user]\nname = "t"\nemail = "t@t"\n' >"$JJ_CONFIG"
q() { "$@" >/dev/null 2>&1; }
echo 0 >"$dir/verdict"
# xb as jj-push.sh sees it: the command runs in a copy of the worktree's tracked and unignored
# files, with each --with DIR=PATH copied in and a leading `nix develop DIR -c` taken off; its
# arguments are kept in xb-argv, and the command after its own options in xb-command
xb=$dir/.local/state/xb/current/bin/xb
mkdir -p "${xb%/*}"
cat >"$xb" <<EOF
#!/usr/bin/env bash
printf '%s\\n' "\$*" >"$dir/xb-argv"
job=\$(mktemp -d -p "$dir")
GIT_DIR=\$(jj --ignore-working-copy git root) GIT_WORK_TREE=\$PWD GIT_INDEX_FILE=\$job.none \\
  git ls-files -zo --exclude-standard | xargs -0r cp -a --parents -t "\$job"
while [[ \$1 == --* ]]; do
  [[ \$1 != --with ]] || { mkdir -p "\$job/\${2#*=}"; cp -a "\${2%%=*}/." "\$job/\${2#*=}"; }
  shift 2
done
printf '%s\\n' "\$*" >"$dir/xb-command"
cd "\$job"
args=("\$@")
[[ \${args[0]} == env ]] && { i=1; while [[ \${args[i]} == *=* ]]; do ((i++)); done
  [[ \${args[i]} == nix ]] && args=("\${args[@]:0:i}" "\${args[@]:i+4}"); }
exec "\${args[@]}"
EOF
chmod +x "$xb"

# a pre-push hook at $1 that keeps its arguments and stdin and exits with the verdict file's number
hook() {
  mkdir -p "${1%/*}"
  printf '#!/bin/sh\necho "$1 $2" >%s/args\ncat >%s/stdin\nexit "$(cat %s/verdict)"\n' "$dir" "$dir" "$dir" >"$1"
  chmod +x "$1"
}
# repo $1 with a bare origin holding main, after `setup` runs in it with the hook to commit
repo() {
  git init -q --bare "$dir/$1.git"
  q jj git init ${init:+"$init"} "$dir/$1"
  q jj -R "$dir/$1" git remote add origin "$dir/$1.git"
  echo a >"$dir/$1/a"
  [ -z "${2-}" ] || "$2" "$dir/$1"
  q jj -R "$dir/$1" commit -m a
  q jj -R "$dir/$1" bookmark create main -r @-
  q jj -R "$dir/$1" git push -b main
}
ships() { hook "$1/.githooks/pre-push"; }
repo hooked ships
git -C "$dir/hooked" config core.hooksPath .githooks
repo shipped ships
git -C "$dir/shipped" config core.hooksPath "$dir/shipped/.git/hooks"
repo inner
hook "$dir/inner/.git/hooks/pre-push"
git -C "$dir/hooked" config xb.prePushShell ""
git -C "$dir/inner" config xb.prePushShell ""
repo unrouted ships
repo here ships
git -C "$dir/here" config xb.prePushLocal true
repo linked ships
ln -s ../../.githooks/pre-push "$dir/linked/.git/hooks/pre-push"
git -C "$dir/linked" config xb.prePushShell ""
ignores() { echo /.hooks/ >"$1/.gitignore"; hook "$1/.hooks/pre-push"; }
repo ignored ignores
git -C "$dir/ignored" config core.hooksPath .hooks
# with a tracked pre-push at the top too, which an ignore check reading the clone's index would take for the hook
husky() { hook "$1/.husky/_/pre-push"; echo '*' >"$1/.husky/_/.gitignore"; echo top >"$1/pre-push"; }
repo husky husky
git -C "$dir/husky" config core.hooksPath .husky/_
repo outside
hook "$dir/hooks-outside/pre-push"
git -C "$dir/outside" config core.hooksPath "$dir/hooks-outside"
repo dotdot
hook "$dir/hooks-up/pre-push"
git -C "$dir/dotdot" config core.hooksPath ../hooks-up
for r in ignored husky outside dotdot; do git -C "$dir/$r" config xb.prePushShell ""; done
git init -q "$dir/outer"
for r in apart outer/nested; do
  init=--no-colocate repo $r ships
  git --git-dir="$(jj -R "$dir/$r" --ignore-working-copy git root)" config xb.prePushShell ""
done
repo plain
q jj -R "$dir/hooked" workspace add --name second "$dir/second"
mkdir "$dir/elsewhere"

# check WANT CWD COMMAND
check() {
  out=$(jq -cn --arg c "$3" --arg cwd "$2" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' | HOME="$dir" bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %-12s %s\n' "$got" "${2#"$dir"}" "$3" ||
    printf 'FAIL want=%s got=%s %s  %s\n' "$1" "$got" "${2#"$dir"}" "$3"
}
k=$dir/hooked e=$dir/elsewhere

check DENY "$k" 'jj git push'
check DENY "$k" 'jj git push -b main'
check DENY "$k" 'jj --color never git push --bookmark main'
check DENY "$e" 'jj -R ~/hooked git push'
check DENY "$e" "jj --repository=$k git push"
check DENY "$e" 'cd ~/hooked && jj git push --allow-empty-description'
check DENY "$k" 'jj st && jj git push'
check DENY "$k" "sh -c 'jj git push -c @'"
check DENY "$dir/shipped" 'jj git push'
check DENY "$dir/inner" 'jj git push'
check DENY "$dir/second" 'jj git push'
check DENY "$e" 'jj -R "$repo" git push'
check DENY "$e" 'cd "$ws" && jj git push'

check PASS "$k" 'jj git push --dry-run'
check PASS "$k" 'jj git push --help'
check PASS "$k" 'jj git fetch'
check PASS "$k" 'jj log -r push'
check PASS "$k" 'echo jj git push'
check PASS "$k" 'git push origin main'
check PASS "$k" 'bash ~/.claude/hooks/jj-push.sh git push -b main'
check DENY "$dir/plain" 'jj git push'
check DENY "$e" 'jj git push'
check DENY "$e" 'jj -R ~/plain git push'
check DENY "$k" 'git push --no-verify origin main'
check DENY "$k" 'git -C ~/hooked push origin main --no-verify'
check DENY "$k" 'git push --no-verif origin main'
check DENY "$k" 'git push origin --no-veri main'
check PASS "$k" 'git push --no-ver origin main'
check DENY "$k" 'echo main | xargs git push --no-verify origin'
check DENY "$k" 'echo main | xargs -I git jj git push -b git'
check PASS "$k" 'git log --no-verify-not-a-push'
# a git that can't say where hooks live, as a broken config or a stub ahead on PATH makes it (jj-push.sh reads it)
mkdir "$dir/failing-git"
printf '#!/bin/sh\ncase "$*" in *--git-path*) echo "fatal: planted" >&2; exit 128 ;; esac\nexec %s "$@"\n' \
  "$(command -v git)" >"$dir/failing-git/git"
chmod +x "$dir/failing-git/git"

# push NAME WANT-EXIT [ARGS...]: jj-push.sh's exit, as WANT-EXIT says it should be
push() {
  local name=$1 want=$2 got
  shift 2
  (cd "$dir/$name" && HOME=${home:-$dir} bash "$p" "$@") >"$dir/out" 2>&1
  got=$?
  if [ "$want" = 0 ]; then [ "$got" = 0 ]; else [ "$got" != 0 ]; fi &&
    printf 'ok   push %-8s exit %s: %s\n' "$name" "$got" "$*" ||
    printf 'FAIL push %-8s exit %s, want %s: %s\n     %s\n' "$name" "$got" "$want" "$*" "$(cat "$dir/out")"
}
# same WHAT GOT WANT
same() {
  [ "$2" = "$3" ] && printf 'ok   %s\n' "$1" || printf 'FAIL %s\n     got  %s\n     want %s\n' "$1" "$2" "$3"
}
at() { git -C "$dir/$1.git" rev-parse -q --verify "refs/heads/$2" || echo none; }
id() { jj -R "$dir/$1" --ignore-working-copy log --no-graph -r "$2" -T commit_id; }
change() {
  echo "$2" >"$dir/$1/$2"
  q jj -R "$dir/$1" commit -m "$2"
  q jj -R "$dir/$1" bookmark move main --to @-
}
z=0000000000000000000000000000000000000000
reason=$(jq -cn --arg c 'git push --no-verify origin main' --arg cwd "$k" '{tool_name:"Bash", tool_input:{command:$c}, cwd:$cwd}' |
  HOME="$dir" bash "$h" | jq -r .hookSpecificOutput.permissionDecisionReason)
same "a git push's refusal gives git's fix, not jj-push.sh" "$(grep -c jj-push <<<"$reason") $(grep -c 'Push without it' <<<"$reason")" "0 1"

change hooked b
old=$(at hooked main) new=$(id hooked main)
push hooked 0 git push -b main
same "a passing hook's push lands" "$(at hooked main)" "$new"
same "the hook gets the remote and its url" "$(cat "$dir/args")" "origin $dir/hooked.git"
same "the hook's stdin is git's: local ref and id, remote ref and id" "$(cat "$dir/stdin")" "refs/heads/main $new refs/heads/main $old"

change hooked c
echo 1 >"$dir/verdict"
before=$(at hooked main)
push hooked 1 git push -b main
same "a failing hook's push doesn't land" "$(at hooked main)" "$before"
same "the refusal names the hook" "$(grep -c 'pre-push exited 1 on xb (.*), so nothing was pushed' "$dir/out")" 1
echo 0 >"$dir/verdict"

q jj -R "$k" bookmark create feat -r main
push hooked 0 -R "$k" git push -b feat
same "a new bookmark goes as an add" "$(cat "$dir/stdin")" "refs/heads/feat $(id hooked feat) refs/heads/feat $z"
gone=$(at hooked feat)
q jj -R "$k" bookmark delete feat
push hooked 0 git push --deleted
same "a deleted bookmark goes as git's delete" "$(cat "$dir/stdin")" "(delete) $z refs/heads/feat $gone"
same "and is gone from the remote" "$(at hooked feat)" none

q jj -R "$k" describe -m edit
echo edit >"$k/unsnapshotted"
push hooked 0 git push -c @
pushed=$(git -C "$dir/hooked.git" for-each-ref --format='%(objectname)' 'refs/heads/push-*')
same "a working-copy edit is pushed as the commit the hook saw" "$(cut -d' ' -f2 "$dir/stdin")" "$pushed"

push hooked 0 git push -b main
: >"$dir/stdin"
push hooked 0 git push -b main
same "nothing to push runs no hook" "$(cat "$dir/stdin")" ""
GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t git -C "$k" tag -a v2 -m v2 "$(id hooked main)"
q jj -R "$k" git import
push hooked 0 git push -t v2
same "an annotated tag reaches the hook by its tag object, as git feeds it" "$(cat "$dir/stdin")" \
  "refs/tags/v2 $(git -C "$k" rev-parse refs/tags/v2) refs/tags/v2 $z"
GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t git -C "$k" tag -a pkg@2 -m pkg "$(id hooked main)"
q jj -R "$k" git import
push hooked 0 git push -t 'exact:"pkg@2"'
same "a name jj's plan quotes reaches the hook as git names it" "$(cat "$dir/stdin")" \
  "refs/tags/pkg@2 $(git -C "$k" rev-parse refs/tags/pkg@2) refs/tags/pkg@2 $z"
q jj -R "$k" bookmark create -r main '"-n"'
push hooked 0 git push -b 'exact:"-n"'
same "even one that reads as echo's flag" "$(cat "$dir/stdin")" "refs/heads/-n $(id hooked main) refs/heads/-n $z"

q jj -R "$k" new
echo wc >"$k/wc"
q jj -R "$k" describe -m wc
q jj -R "$k" bookmark create sealed -r @-
q jj -R "$k" bookmark create wc -r @
push hooked 0 --config 'git.private-commits="working_copies()"' git push --all
same "a push of every bookmark sends all but the private one, which jj skips with a warning" "$(at hooked sealed) $(at hooked wc)" "$(id hooked sealed) none"
same "and the hook is fed only what is sent" "$(cut -d' ' -f1 "$dir/stdin")" refs/heads/sealed

change plain b
push plain 0 git push -b main
same "a repo with no hook pushes as jj does" "$(at plain main)" "$(id plain main)"
# the width rule: a commit not on the remote yet with a long first line is refused, named by its change, an old one
# on the remote isn't read, and a reworded one goes through
over="x: $(printf 'a%.0s' {1..70})"
q jj -R "$dir/plain" describe -r main -m "$over" --ignore-immutable
q jj -R "$dir/plain" git push -b main
change plain wide
q jj -R "$dir/plain" describe -r main -m "$over"
before=$(at plain main)
push plain 1 git push -b main
wide=$(jj -R "$dir/plain" --ignore-working-copy log --no-graph -r main -T 'change_id.shortest(8)')
same "a long first line not on the remote is refused, by change" \
  "$(at plain main) $(grep -c "  $wide: its first line is 73 characters" "$dir/out") $(grep -c "jj describe -r <change>" "$dir/out")" \
  "$before 1 1"
push plain 1 --quiet git push -b main
same "a quiet push is read all the same" "$(at plain main) $(grep -c "  $wide: its first line" "$dir/out")" "$before 1"
push plain 1 --config ui.quiet=true git push -b main
same "one quiet by config is refused unread" "$(at plain main) $(grep -c "doesn't say nothing changed" "$dir/out")" "$before 1"
q jj -R "$dir/plain" describe -r main -m 'x: short'
push plain 0 git push -b main
same "the reworded one goes through, the old long one on the remote unread" "$(at plain main)" "$(id plain main)"
git -C "$dir/plain" tag v1 "$(id plain main)"
q jj -R "$dir/plain" git import
push plain 0 git push -t v1
same "a tag goes through the same check" "$(git -C "$dir/plain.git" rev-parse -q --verify refs/tags/v1)" "$(id plain main)"
git -C "$dir/plain" tag pkg@1.2.3 "$(id plain main)"
q jj -R "$dir/plain" git import
push plain 0 git push -t 'exact:"pkg@1.2.3"'
same "so does one jj's plan quotes" "$(git -C "$dir/plain.git" rev-parse -q --verify refs/tags/pkg@1.2.3)" "$(id plain main)"
change plain c
before=$(at plain main)
PATH=$dir/failing-git:$PATH push plain 1 git push -b main
same "but not when git can't say whether it has one, and git's words are given" \
  "$(at plain main) $(grep -c "git can't say where .* keeps its hooks: fatal: planted, so nothing was pushed" "$dir/out")" \
  "$before 1"

repo twice ships
git -C "$dir/twice" config xb.prePushShell ""
git init -q --bare "$dir/fork.git"
q jj -R "$dir/twice" git remote add fork "$dir/fork.git"
q jj -R "$dir/twice" bookmark track main@fork
change twice b
before=$(at twice main)
push twice 1 git push --remote origin --remote fork -b main
same "a hooked push to two remotes is refused, since git runs the hook once per remote" \
  "$(at twice main) $(grep -c 'push to one at a time' "$dir/out")" "$before 1"
push twice 0 git push --remote fork -b main
# jj 0.46 itself then fails, running `git push fork` with nothing to send ("You are not currently on a branch")
push twice 1 git push --remote origin --remote fork -b main
same "a remote with nothing to send doesn't count, and the hook gets the other's lines" \
  "$(grep -c 'one at a time' "$dir/out") $(cat "$dir/stdin")" "0 refs/heads/main $(id twice main) refs/heads/main $before"

git init -q --bare "$dir/é-fork.git"
q jj git init "$dir/accent"
q jj -R "$dir/accent" git remote add é-fork "$dir/é-fork.git"
git -C "$dir/accent" config xb.prePushShell ""
ships "$dir/accent"
q jj -R "$dir/accent" commit -m a
q jj -R "$dir/accent" bookmark create main -r @-
push accent 0 git push --remote é-fork -b main
same "a remote jj's plan quotes reaches the hook as git names it, with its push url" "$(cat "$dir/args")" \
  "é-fork $dir/é-fork.git"
git init -q --bare "$dir/dash.git"
q jj git init "$dir/dash"
q jj -R "$dir/dash" git remote add -- -x "$dir/dash.git"
git -C "$dir/dash" config xb.prePushShell ""
ships "$dir/dash"
q jj -R "$dir/dash" commit -m a
q jj -R "$dir/dash" bookmark create main -r @-
push dash 0 git push --remote='exact:"-x"' -b main
same "so does one named like an option" "$(cat "$dir/args")" "-x $dir/dash.git"

repo mirrored ships
git -C "$dir/mirrored" config xb.prePushShell ""
git init -q --bare "$dir/mirror.git"
git -C "$dir/mirrored" remote set-url --add --push origin "$dir/mirrored.git"
git -C "$dir/mirrored" remote set-url --add --push origin "$dir/mirror.git"
change mirrored b
before=$(at mirrored main)
push mirrored 1 git push -b main
same "a remote with two push URLs is refused, since git runs the hook once per URL" \
  "$(at mirrored main) $(grep -c 'pushes to 2 URLs' "$dir/out")" "$before 1"

init=--no-colocate repo apart ships
GIT_DIR=$(jj -R "$dir/apart" --ignore-working-copy git root) git config xb.prePushShell ""
q jj -R "$dir/apart" tag set t1 -r main
push apart 0 git push -t t1
same "a tag jj made, which git's store doesn't hold, reaches the hook as its commit" "$(cat "$dir/stdin")" \
  "refs/tags/t1 $(id apart main) refs/tags/t1 $z"

change hooked d
push hooked 0 git push -b main
same "a hook the worktree tracks runs on xb from its own path" "$(cut -d' ' -f1-4 "$dir/xb-command")" "env bash -c remote=\$1"
same "with its own path in the tree" "$(grep -o '\./\.githooks/pre-push' "$dir/xb-argv")" ./.githooks/pre-push
change inner d
push inner 0 git push -b main
same "a hook under .git is shipped beside the tree" "$(grep -o -- '--with [^ ]*' "$dir/xb-argv")" "--with $dir/inner/.git/hooks=.xb-pre-push"
same "and fed as git feeds it" "$(cat "$dir/args")" "origin $dir/inner.git"
git -C "$k" config xb.prePushShell ./formalization
git -C "$k" config --add xb.prePushEnv SKIP_LEAN
git -C "$k" config --add xb.prePushEnv UNSET_HERE
change hooked e
SKIP_LEAN=1 push hooked 0 git push -b main
same "a clone's shell and the variables it names go with the job, a set one only" \
  "$(cut -d' ' -f1-6 "$dir/xb-command")" "env SKIP_LEAN=1 nix develop ./formalization -c"
same "and the hook still gets git's stdin" "$(cut -d' ' -f1 "$dir/stdin")" refs/heads/main

change unrouted b
before=$(at unrouted main)
push unrouted 1 git push -b main
same "a hooked clone that chose neither xb nor here is refused" "$(at unrouted main)" "$before"
same "and told how to choose" "$(grep -c 'git config xb.prePushShell DIR' "$dir/out")" 1
printf '[xb]\n\tprePushLocal = true\n' >"$dir/global"
GIT_CONFIG_GLOBAL=$dir/global push unrouted 1 git push -b main
same "the choice is the clone's: a global one doesn't count" "$(at unrouted main)" "$before"
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=xb.prePushShell GIT_CONFIG_VALUE_0= push unrouted 1 git push -b main
same "nor does one in the environment" "$(at unrouted main)" "$before"
git init -q --bare "$dir/decoy.git"
git --git-dir="$dir/decoy.git" config xb.prePushShell ""
GIT_DIR=$dir/decoy.git GIT_WORK_TREE=$dir/unrouted push unrouted 1 git push -b main
same "nor one in a repo a caller's GIT_DIR and GIT_WORK_TREE name" "$(at unrouted main)" "$before"
change inner e
echo 1 >"$dir/verdict"
before=$(at inner main)
GIT_DIR=$dir/decoy.git GIT_WORK_TREE=$dir/inner push inner 1 git push -b main
same "and those don't hide the clone's own hook" "$(at inner main)" "$before"
echo 0 >"$dir/verdict"
rm -f "$dir/xb-argv"
change here b
push here 0 git push -b main
same "a clone that runs its hook here doesn't call xb" "$([ -e "$dir/xb-argv" ] && echo called || echo not)" not
same "and the hook gets git's arguments" "$(cat "$dir/args")" "origin $dir/here.git"
change linked b
push linked 0 git push -b main
same "a hook linked to a tracked file runs from the file's path, unshipped" \
  "$(grep -c -- '--with' "$dir/xb-argv") $(grep -o '\./\.githooks/pre-push' "$dir/xb-argv")" "0 ./.githooks/pre-push"
echo s >"$dir/second/s"
q jj -R "$dir/second" commit -m s
q jj -R "$dir/second" bookmark create side -r @-
push second 0 git push -b side
same "a second workspace runs its tracked hook from its own path, unshipped" \
  "$(grep -c -- '--with' "$dir/xb-argv") $(grep -o '\./\.githooks/pre-push' "$dir/xb-argv") $(at hooked side)" \
  "0 ./.githooks/pre-push $(id second side)"
change apart b
push apart 0 git push -b main
same "so does a repo whose store is inside .jj, with no .git" \
  "$(grep -c -- '--with' "$dir/xb-argv") $(grep -o '\./\.githooks/pre-push' "$dir/xb-argv") $(at apart main)" \
  "0 ./.githooks/pre-push $(id apart main)"
change outer/nested b
push outer/nested 0 git push -b main
same "and one inside another git checkout, which git alone would take it for" \
  "$(at outer/nested main)" "$(id outer/nested main)"
change ignored b
push ignored 0 git push -b main
same "a hook the worktree ignores is shipped beside the tree" "$(grep -o -- '--with [^ ]*' "$dir/xb-argv")" "--with $dir/ignored/.hooks=.xb-pre-push"
same "and runs there" "$(cat "$dir/args")" "origin $dir/ignored.git"
change outside b
push outside 0 git push -b main
same "a hook outside the worktree is shipped beside the tree" "$(grep -o -- '--with [^ ]*' "$dir/xb-argv")" "--with $dir/hooks-outside=.xb-pre-push"
change dotdot b
push dotdot 0 git push -b main
same "a hook reached through .. is shipped from where it is" "$(grep -o -- '--with [^ ]*' "$dir/xb-argv")" "--with $dir/hooks-up=.xb-pre-push"
change husky b
before=$(at husky main)
push husky 1 git push -b main
same "a hook its own directory ignores, which xb would ship without it, is refused" \
  "$(at husky main) $(grep -c 'ignored in its own directory' "$dir/out")" "$before 1"
change hooked f
home=$dir/nowhere push hooked 1 git push -b main
same "with no xb here the push is refused" "$(grep -c 'there is no' "$dir/out")" 1
git -C "$k" config --add xb.prePushEnv url
SKIP_LEAN=1 url=mine push hooked 0 git push -b main
same "a named variable is the caller's, not one of jj-push.sh's own" "$(cut -d' ' -f1-3 "$dir/xb-command")" "env SKIP_LEAN=1 url=mine"
git -C "$k" config branch.main.remote origin
git -C "$k" config branch.main.merge refs/heads/main
change hooked state
new=$(id hooked main)
push hooked 0 git push -b main
same "the job's repo gets the pushed tip, the remote's ref for it and the branch's upstream" \
  "$(grep -o -- '--git-ref [^ ]*' "$dir/xb-argv" | sort | tr '\n' ' ')" \
  "--git-ref refs/heads/main=$new --git-ref refs/remotes/origin/main "
same "with the config naming that upstream" \
  "$(grep -o -- '--git-config [^ ]*' "$dir/xb-argv" | sort | tr '\n' ' ')" \
  "--git-config branch.main.merge=refs/heads/main --git-config branch.main.remote=origin --git-config remote.origin.fetch=+refs/heads/*:refs/remotes/origin/* "
change hooked g
git -C "$k" config --add xb.prePushEnv 'a.b'
push hooked 1 git push -b main
same "a name in xb.prePushEnv that names no variable is refused" "$(grep -c "which is no variable's name" "$dir/out")" 1
git -C "$k" config --replace-all xb.prePushEnv 'a[$(touch pwned)]' '^a\.b$'
push hooked 1 git push -b main
same "as is one that would run code" \
  "$(grep -c "which is no variable's name" "$dir/out") $([ -e "$k/pwned" ] && echo ran || echo not)" "1 not"
