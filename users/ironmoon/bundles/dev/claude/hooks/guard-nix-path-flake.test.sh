h="${1:-$HOME/.claude/hooks/guard-nix-path-flake.sh}"
NOGIT=$(mktemp -d)
GIT=$(mktemp -d)
git -C "$GIT" init -q
mkdir -p "$NOGIT/sub" "$GIT/sub"
SP=$(realpath "$(command -v bash)")
SP=${SP%/bin/bash}
ln -s "$SP" "$NOGIT/result"
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

check DENY "$NOGIT" 'true && nix develop .'
check DENY "$NOGIT" 'true; nix develop .#crates -c cargo test'
check DENY "$NOGIT" 'env FOO=1 nix develop .'
check DENY "$NOGIT" 'sudo nix build .#default'
check DENY "$NOGIT" 'x=$(nix eval .#foo)'
check DENY "$NOGIT" 'echo "$(nix develop -c true)"'
check DENY "$NOGIT" '~/tmp/np-heavy.sh nix develop .#crates -c cargo test'
check DENY "$NOGIT" 'nice -n 5 nix build .'
check DENY "$NOGIT" 'echo . | xargs nix build'
check DENY "$NOGIT" 'bash -lc "nix develop ."'
check DENY "$NOGIT" "nix develop $GIT -c bash -c 'nix develop .'"
check DENY "$NOGIT" "(cd $GIT && ls); nix develop"
check DENY "$GIT" "env -C $NOGIT nix develop"
check DENY "$GIT" "cd $NOGIT && cd - && nix develop"
check DENY "$NOGIT" "echo 'nix develop' && nix develop ."

check PASS "$NOGIT" "tools/mem.py add 'run crate tests with nix develop .#crates -c cargo test'"
check PASS "$NOGIT" "echo 'see nix develop .#crates'"
check PASS "$NOGIT" 'git commit -m "nix develop . here; then nix build"'
check PASS "$NOGIT" "rg 'nix develop' notes"
check PASS "$NOGIT" 'ls  # then nix develop'
check PASS "$NOGIT" 'cat <<END > notes.md
nix develop .
END'
check PASS "$NOGIT" "(cd $GIT && nix develop -c true)"
check PASS "$NOGIT" "nix eval --apply 'x: x' nixpkgs#lib.version"
check PASS "$NOGIT" "nix build --impure --no-link --expr '(import /abs/path/run.nix)'"
check PASS "$NOGIT" 'nix build -f ./default.nix'
check PASS "$NOGIT" 'nix develop --file shell.nix -c true'
check DENY "$NOGIT" "nix build -f default.nix --override-input dep path:$NOGIT"

check PASS "$NOGIT" "nix profile add $SP"
check PASS "$NOGIT" "nix build $SP^out"
check PASS "$NOGIT" "nix run $SP"
check PASS "$NOGIT" "nix shell $SP/bin -c true"
check PASS "$NOGIT" 'nix build ./result'
check PASS "$GIT" "sudo -u agent sh -c 'nix profile add $SP'"
check DENY "$GIT" "sudo -u agent sh -c 'cd $NOGIT && nix develop'"
check DENY "$NOGIT" 'nix profile add .'
check DENY "$NOGIT" "nix build path:$SP"
check DENY "$NOGIT" "nix build $SP#default"
check DENY "$NOGIT" "nix build $SP/bin^out"
check DENY "$NOGIT" 'nix build ./result/bin'
check DENY "$GIT" "nix build $NOGIT^out"
