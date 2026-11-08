# Who a commit names, by jj and by git: the agent in a listed repo under a session, the base identity elsewhere.
# Takes the config dir holding jj/conf.d/claude.toml and git/claude-identity{,.user}, and the settings.json whose env a
# session gets.
config=$(realpath "${1:-$HOME/.config}")
settings=$(realpath "${2:-$HOME/.claude/settings.json}")
agent="ironmoon's agent <agent@ironmoon.dev>"
base="Base <base@example.com>"
dir=$(realpath "$(mktemp -d)")
trap 'rm -r "${dir:?}"' EXIT
mkdir -p "$dir/.config/jj/conf.d" "$dir/.config/git" "$dir/workspace"
cp -L "$config/jj/conf.d/claude.toml" "$dir/.config/jj/conf.d/"
cp -L "$config/git/claude-identity" "$config/git/claude-identity.user" "$dir/.config/git/"
printf '[user]\nname = "Base"\nemail = "base@example.com"\n' >"$dir/.config/jj/config.toml"
printf '[user]\n\tname = Base\n\temail = base@example.com\n' >"$dir/.config/git/config"
mapfile -t session < <(jq -r '.env | to_entries[] | select(.key | startswith("GIT_CONFIG_")) | "\(.key)=\(.value)"' "$settings")
session+=(CLAUDECODE=1)

run() { env -i PATH="$PATH" HOME="$dir" "$@"; }
jj_author() {
  run "${@:2}" jj --quiet git init "$1" &&
    run "${@:2}" jj --quiet -R "$1" commit -m test &&
    run "${@:2}" jj --quiet -R "$1" log --no-graph -r @- -T 'author.name() ++ " <" ++ author.email() ++ ">"'
}
git_author() {
  run "${@:2}" git init -q "$1" &&
    run "${@:2}" git -C "$1" commit -q --allow-empty -m test &&
    run "${@:2}" git -C "$1" log -1 --format='%an <%ae>'
}

failed=0
# check WANT TOOL REPO [ENV...]
check() {
  repo="$dir/$3/$2${4:+-session}"
  mkdir -p "$repo"
  got=$("${2}_author" "$repo" "${@:4}" 2>"$dir/stderr")
  [ "$got" = "$1" ] && echo "ok   $2 $3 ${4:+session}: $got" ||
    { failed=1; echo "FAIL $2 $3 ${4:+session}: want $1, got $got"; cat "$dir/stderr"; }
}
for tool in jj git; do
  check "$agent" $tool workspace/claude "${session[@]}"
  check "$base" $tool workspace/claude
  check "$base" $tool elsewhere "${session[@]}"
done
exit $failed
