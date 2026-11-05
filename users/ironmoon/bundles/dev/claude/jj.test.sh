c="${1:-$HOME/.config/jj/conf.d/claude.toml}"
# jj skips a missing config path without a word
[ -f "$c" ] || { echo "FAIL no config at $c"; exit 1; }
dir=$(realpath "$(mktemp -d)")
trap 'rm -rf "${dir:?}"' EXIT
cd "$dir" || exit 1
status=0
printf '[user]\nname = "t"\nemail = "t@t"\n' >"$dir/user.toml"
export JJ_CONFIG="$dir/user.toml:$c" GIT_CONFIG_GLOBAL=/dev/null
unset CLAUDECODE
q() { "$@" >/dev/null 2>&1; }
r=$dir/repo
git init -q --bare "$r.git"
q jj git init "$r"
q jj -R "$r" git remote add origin "$r.git"
q jj -R "$r" workspace add --name second "$dir/second"

# edit WS FILE: workspace WS's @ adds FILE and carries bookmark b
edit() {
  echo "$2" >"$1/$2"
  q jj -R "$1" describe -m "$2"
  q jj -R "$1" bookmark set b -r @ --allow-backwards
}
at() { git -C "$r.git" rev-parse -q --verify refs/heads/b || echo none; }
id() { jj -R "$r" --ignore-working-copy log --no-graph -r b -T commit_id; }
# check WANT WHAT COMMAND...: after COMMAND, whether the remote's b is the local one
check() {
  local want=$1 what=$2 got
  shift 2
  q "$@"
  [ "$(at)" = "$(id)" ] && got=pushed || got=refused
  [ "$got" = "$want" ] && printf 'ok   %-7s %s\n' "$got" "$what" || {
    printf 'FAIL %s, want %s: %s\n' "$got" "$want" "$what"
    status=1
  }
}
session() { CLAUDECODE=1 jj -R "$r" git push -b b "$@"; }

edit "$r" a
check refused "a session pushing its @" session
check pushed "a push outside a session" jj -R "$r" git push -b b
edit "$r" b
q jj -R "$r" new
check pushed "a session pushing what it sealed with \`jj new\`" session
edit "$dir/second" c
check refused "a session pushing another workspace's @" session
check pushed "a session pushing with --allow-private" session --allow-private
exit "$status"
