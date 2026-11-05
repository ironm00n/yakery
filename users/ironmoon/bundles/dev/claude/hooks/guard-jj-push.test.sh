h="${1:-$HOME/.claude/hooks/guard-jj-push.sh}"
p="${2:-${h%/*}/jj-push.sh}"
dir=$(realpath "$(mktemp -d)")
trap 'rm -rf "${dir:?}"' EXIT
export JJ_CONFIG="$dir/jj.toml" GIT_CONFIG_GLOBAL=/dev/null
printf '[user]\nname = "t"\nemail = "t@t"\n' >"$JJ_CONFIG"
q() { "$@" >/dev/null 2>&1; }
echo 0 >"$dir/verdict"

# a pre-push hook at $1 that keeps its arguments and stdin and exits with the verdict file's number
hook() {
  mkdir -p "${1%/*}"
  printf '#!/bin/sh\necho "$1 $2" >%s/args\ncat >%s/stdin\nexit "$(cat %s/verdict)"\n' "$dir" "$dir" "$dir" >"$1"
  chmod +x "$1"
}
# repo $1 with a bare origin holding main, after `setup` runs in it with the hook to commit
repo() {
  git init -q --bare "$dir/$1.git"
  q jj git init "$dir/$1"
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
check PASS "$dir/plain" 'jj git push'
check PASS "$e" 'jj git push'
check PASS "$e" 'jj -R ~/plain git push'

# push NAME WANT-EXIT [ARGS...]: jj-push.sh's exit, as WANT-EXIT says it should be
push() {
  local name=$1 want=$2 got
  shift 2
  (cd "$dir/$name" && bash "$p" "$@") >"$dir/out" 2>&1
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
same "the refusal names the hook" "$(grep -c 'pre-push exited 1, so nothing was pushed' "$dir/out")" 1
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
