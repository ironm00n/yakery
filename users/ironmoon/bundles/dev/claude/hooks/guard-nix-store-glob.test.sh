h="${1:-$HOME/.claude/hooks/guard-nix-store-glob.sh}"
TMP=$(realpath "$(mktemp -d)")
trap 'rm -rf -- "$TMP"' EXIT
ln -s /nix/store "$TMP/store"
ONE=$(realpath "$(command -v bash)")
ONE=${ONE%/bin/bash}

check() {
  out=$(jq -cn --arg c "$3" --arg d "$2" '{tool_name:"Bash",tool_input:{command:$c},cwd:$d}' | bash "$h")
  got=$([ -n "$out" ] && echo DENY || echo PASS)
  [ "$got" = "$1" ] && printf 'ok   %-4s %s\n' "$got" "$3" || printf 'FAIL want=%s got=%s  %s\n' "$1" "$got" "$3"
}

check DENY "$TMP" 'ls -d /nix/store/*/lib/rustlib/rustc-src/rust/compiler/rustc_const_eval 2>/dev/null | head'
check DENY "$TMP" 'ls /nix/store/*rust*'
check DENY "$TMP" 'echo /nix/store/?a*'
check DENY "$TMP" 'ls -d /nix/store/[a-f]*-jq-*'
check DENY "$TMP" 'ls -d /nix/store/**/bin'
check DENY "$TMP" 'ls -d /nix/store/abc*/bin'
check DENY "$TMP" 'ls /nix//store/*'
check DENY "$TMP" 'for p in /nix/store/*-jq-*; do echo $p; done'
check DENY "$TMP" 'cd /nix/store && ls -d *jq*'
check DENY /nix/store 'ls -d *jq*'
check DENY "$TMP" 'cd /nix && ls -d store/*'
check DENY "$TMP" 'ls -d store/*'
check DENY "$TMP" "ls -d $TMP/store/*"
check DENY "$TMP" "sh -c 'ls -d /nix/store/*'"
check DENY "$TMP" 'x=$(ls -d /nix/store/*-jq-*)'
check DENY "$TMP" 'sudo ls -d /nix/store/*'
check DENY "$TMP" 'echo x | xargs ls -d /nix/store/*'
check DENY "$TMP" 'true; nix-store -q --references /nix/store/*-jq-*.drv'

check PASS "$TMP" "rg -l jq '/nix/store/*'"
check PASS "$TMP" 'echo "/nix/store/*"'
check PASS "$TMP" "rg -g '*.nix' jq /nix/store/abc-src"
check PASS "$TMP" 'ls /nix/store/\*'
check PASS "$TMP" 'fd -d 1 jq /nix/store'
check PASS "$TMP" 'ls /nix/store | rg jq'
check PASS "$TMP" "ls $ONE/*"
check PASS "$TMP" "cd $ONE && ls *"
check PASS "$TMP" 'ls -d /nix/store*'
check PASS "$TMP" 'ls -d /*'
check PASS "$TMP" 'ls $d/*'
check PASS "$TMP" 'cd "$d" && ls *'
check PASS "$TMP" '[[ $p == /nix/store/* ]] && echo y'
check PASS "$TMP" 'case $p in /nix/store/*) echo y ;; esac'
check PASS "$TMP" 'case $p in a) echo a ;; /nix/store/*) echo y ;; esac'
check PASS "$TMP" "cat <<'EOF'
/nix/store/*
EOF"
check PASS "$TMP" "nix path-info $ONE"
check PASS "$TMP" 'echo hi'
