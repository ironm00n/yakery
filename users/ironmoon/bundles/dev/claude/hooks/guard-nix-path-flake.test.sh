h="${1:-$HOME/.claude/hooks/guard-nix-path-flake.sh}"
NOGIT=$(mktemp -d)
GIT=$(mktemp -d)
git -C "$GIT" init -q
mkdir -p "$NOGIT/sub" "$GIT/sub"
trap 'rm -rf -- "$NOGIT" "$GIT"' EXIT

check() {
  out=$(jq -cn --arg c "$3" --arg d "$2" '{tool_name:"Bash",tool_input:{command:$c},cwd:$d}' | bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %s\n' "$got" "$3" || printf 'FAIL want=%s got=%s  %s\n' "$1" "$got" "$3"
}

check DENY "$NOGIT" 'nix develop -c cargo test'
check DENY "$NOGIT" 'nix develop'
check DENY "$NOGIT" 'nix build .#default'
check DENY "$NOGIT" 'nix flake check'
check DENY "$NOGIT" 'nix profile install .#tool'
check DENY "$NOGIT" 'nix print-dev-env --json'
check DENY "$GIT" "nix develop $NOGIT -c true"
check DENY "$GIT" "cd $NOGIT && nix develop -c lake build"
check DENY "$GIT" "sh -c 'cd $NOGIT/sub && nix develop'"
check DENY "$GIT" "nix develop path:$GIT"
check DENY "$GIT" "nix build --override-input dep path:$NOGIT"
check DENY "$GIT" "direnv exec $NOGIT lake build"
check DENY "$GIT" 'cd "$WS" && nix develop'

check PASS "$GIT" 'nix develop -c cargo test'
check PASS "$GIT/sub" 'nix build .#default'
check PASS "$NOGIT" "nix develop $GIT -c sh -c 'cd $NOGIT && cargo test'"
check PASS "$NOGIT" "nix develop git+file://$GIT?rev=abc -c true"
check PASS "$NOGIT" 'nix shell nixpkgs#elan -c elan toolchain list'
check PASS "$NOGIT" 'nix eval --raw nixpkgs#path'
check PASS "$NOGIT" 'nix run github:owner/repo -- --help'
check PASS "$NOGIT" "direnv exec $GIT true"
check PASS "$NOGIT" '~/.elan/bin/lake build'
check PASS "$NOGIT" 'nixos-rebuild build'
check PASS "$NOGIT" 'ls /etc/nixos'
check PASS "$NOGIT" 'nix store gc --dry-run'
