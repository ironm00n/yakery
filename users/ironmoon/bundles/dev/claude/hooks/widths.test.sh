#!/usr/bin/env bash
# lib/widths.sh's rule on messages, the commits it reads from a push, lib/provenance.sh's rule, and git-pre-push.sh in a
# real git push.
hooks=${1:-$HOME/.claude/hooks}
dir=$(realpath "$(mktemp -d)")
trap 'rm -rf "${dir:?}"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
# the agent identity the provenance rule reads, as the claude bundle writes it
export XDG_CONFIG_HOME=$dir/config
mkdir -p "$XDG_CONFIG_HOME/git" && printf '[user]\n\temail = agent@t\n' >"$XDG_CONFIG_HOME/git/claude-identity.user"
ok=0 failed=0
same() {
  if [[ $2 == "$3" ]]; then ((ok++)); printf 'ok   %s\n' "$1"; else ((failed++)); printf 'FAIL %s\n     got  %s\n     want %s\n' "$1" "$2" "$3"; fi
}
repeat() { local s; printf -v s "%$2s" ''; printf %s "${s// /$1}"; }
fits="x: $(repeat a 69)" over="x: $(repeat a 70)"
body="$(repeat 'word ' 14)wo" long_body="$(repeat 'word ' 14)wor"
url="https://example.org/$(repeat p 70)"

# rule MESSAGE: what commit-width.jq finds, compact, or "fits"
rule() {
  local out
  out=$(printf 'c\n%s' "$1" | jq -Rsr --argjson width 72 -f "$hooks/lib/commit-width.jq") || { echo error; return; }
  out=${out#c }
  echo "${out:-fits}"
}
same "a first line of 72 fits" "$(rule "$fits")" fits
same "one of 73 doesn't" "$(rule "$over")" '{"subject":73,"lines":[],"passed":0}'
same "characters, not bytes" "$(rule "x: $(repeat é 69)")" fits
same "a body line of 72 fits" "$(rule $'x: y\n\n'"$body")" fits
same "each body line over is named" "$(rule $'x: y\n\n'"$long_body"$'\n'"$body"$'\n'"$long_body")" \
  '{"subject":null,"lines":[3,5],"passed":0}'
same "both at once" "$(rule "$over"$'\n\n'"$long_body")" '{"subject":73,"lines":[3],"passed":0}'
same "a URL alone passes" "$(rule $'x: y\n\nsee '"$url")" fits
same "a URL in breakable prose doesn't" "$(rule $'x: y\n\nsee '"$url $long_body")" '{"subject":null,"lines":[3],"passed":0}'
same "a long code span alone passes" "$(rule $'x: y\n\n`/nix/store/'"$(repeat h 70)"'`')" fits
same "the closing trailers pass" "$(rule $'x: y\n\n'"$body"$'\n\nAssisted-by: m '"$(repeat c 70)")" fits
same "trailers need a key: value first line" "$(rule $'x: y\n\n  - '"$long_body")" '{"subject":null,"lines":[3],"passed":0}'
same "a fenced block passes and closes" "$(rule $'x: y\n\n```\n'"$long_body"$'\n```\n'"$long_body")" \
  '{"subject":null,"lines":[6],"passed":1}'
same "trailing blanks don't count" "$(rule $'x: y\n\n'"$body"'          ')" fits
same "a leading blank line is skipped" "$(rule $'\n'"$over")" '{"subject":73,"lines":[],"passed":0}'
same "an empty message fits" "$(rule '')" fits
same "a fence opens only in the body" "$(rule $'```x```: y\n\n'"$long_body")" '{"subject":null,"lines":[3],"passed":0}'

# a repo with a bare origin: base is pushed, old is a long commit already on the remote, then new commits on top
git init -q --bare "$dir/origin.git"
git init -q -b main "$dir/work"
cd "$dir/work" || exit 1
git remote add origin "$dir/origin.git"
git commit -q --allow-empty -m 'x: base'
git commit -q --allow-empty -m "$over"
git push -q origin main
old=$(git rev-parse HEAD)
git commit -q --allow-empty -m 'x: fine'
git commit -q --allow-empty -m "$over"
wide=$(git rev-parse HEAD)
git commit -q --allow-empty -m $'x: y\n\n'"$long_body"
tip=$(git rev-parse HEAD)
zero=0000000000000000000000000000000000000000
. "$hooks/lib/widths.sh"
found=$(echo "refs/heads/main $tip refs/heads/main $old" | too_wide)
same "only commits not on the remote are read" "$found" \
  "$tip {\"subject\":null,\"lines\":[3],\"passed\":0}"$'\n'"$wide {\"subject\":73,\"lines\":[],\"passed\":0}"
same "a new branch reads from the remote's refs" \
  "$(echo "refs/heads/topic $tip refs/heads/topic $zero" | too_wide | wc -l)" 2
git init -q --bare "$dir/fork.git"
git remote add fork "$dir/fork.git"
same "a commit on another remote's refs isn't read again" \
  "$(echo "refs/heads/main $old refs/heads/main $zero" | too_wide)" ""
same "a deleted branch sends nothing" "$(echo "(delete) $zero refs/heads/main $old" | too_wide)" ""
reason=$(too_wide_reason "$found" echo "Fix them.")
same "the reason names each commit and what's over" "$(sed -n 2,3p <<<"$reason")" \
  "  $tip: body line 3 runs over"$'\n'"  $wide: its first line is 73 characters"

# git runs the hook from config beside the repo's own, and it acts only in a Claude Code session
git config hook.commit-rules.event pre-push
git config hook.commit-rules.command "$hooks/git-pre-push.sh"
env -u CLAUDECODE git push -q origin main 2>"$dir/err"
same "outside a session a wide push goes through" "$? $(git rev-parse origin/main)" "0 $tip"
git reset -q --hard "$old"
git push -q -f origin main
git commit -q --allow-empty -m "$over"
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "in a session it's refused" "$? $(git rev-parse origin/main)" "1 $old"
same "naming the commit" "$(grep -c "$(git rev-parse --short HEAD): its first line is 73 characters" "$dir/err")" 1
CLAUDECODE= git push -q origin main 2>"$dir/err"
same "an empty CLAUDECODE is still a session" "$? $(git rev-parse origin/main)" "1 $old"
git commit -q --amend --allow-empty -m 'x: short'
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "a reworded one goes through" "$? $(git rev-parse origin/main)" "0 $(git rev-parse HEAD)"
# a signature git log would print first isn't read as the first line
ssh-keygen -q -t ed25519 -N '' -f "$dir/key"
git config gpg.format ssh
git config user.signingkey "$dir/key.pub"
git config log.showSignature true
printf '%s %s\n' t@t "$(cat "$dir/key.pub")" >"$dir/allowed"
git config gpg.ssh.allowedSignersFile "$dir/allowed"
git commit -q -S --allow-empty -m 'x: signed'
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "a signed commit is read by its message" "$? $(git rev-parse origin/main)" "0 $(git rev-parse HEAD)"
git commit -q -S --allow-empty -m "$over"
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "a signed wide one is refused under its own sha, not the signature's line" \
  "$? $(grep -c "$(git rev-parse --short HEAD): its first line is 73 characters" "$dir/err")" "1 1"

# the provenance rule: lib/provenance.sh names each commit not on the remote that is authored as the agent identity
# (agent@t, the identity file's) with no Generated-by trailer, Assisted-by alone included; the hook refuses it in a
# session and passes it outside one
. "$hooks/lib/provenance.sh"
git reset -q --hard origin/main
git commit -q --allow-empty -m 'x: theirs'
base=$(git rev-parse HEAD)
git push -q origin main
GIT_AUTHOR_EMAIL=agent@t git commit -q --allow-empty -m $'x: assisted\n\nAssisted-by: m h'
assisted=$(git rev-parse HEAD)
GIT_AUTHOR_EMAIL=Agent@T git commit -q --allow-empty -m 'x: bare'
bare=$(git rev-parse HEAD)
GIT_AUTHOR_EMAIL=agent@t git commit -q --allow-empty -m $'x: generated\n\nGenerated-by: m h'
git commit -q --allow-empty -m 'x: someone else, no trailer'
tip=$(git rev-parse HEAD)
same "agent commits with no Generated-by are named, case aside, others not" \
  "$(echo "refs/heads/main $tip refs/heads/main $base" | unmarked | sort)" "$(printf '%s\n' "$assisted" "$bare" | sort)"
same "with no identity to read, it fails" "$(echo "refs/heads/main $tip refs/heads/main $base" | XDG_CONFIG_HOME=$dir/none unmarked; echo $?)" 1
env -u CLAUDECODE git push -q --dry-run origin main 2>"$dir/err"
same "outside a session the push goes through" "$?" 0
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "in a session it's refused, naming each" \
  "$? $(git rev-parse origin/main) $(grep -c -e "^  $(git rev-parse --short "$assisted")$" -e "^  $(git rev-parse --short "$bare")$" "$dir/err")" "1 $base 2"
git reset -q --hard "$base"
GIT_AUTHOR_EMAIL=agent@t git commit -q --allow-empty -m $'x: generated\n\nGenerated-by: m h'
CLAUDECODE=1 git push -q origin main 2>"$dir/err"
same "an agent commit with Generated-by goes through" "$? $(git rev-parse origin/main)" "0 $(git rev-parse HEAD)"

echo "widths: ok $ok FAIL $failed"
((failed == 0))
