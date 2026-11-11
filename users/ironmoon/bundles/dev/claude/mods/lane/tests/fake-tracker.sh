# Sourced by tests that read the tracker: a scratch $dir with a token, and on $dir/bin a curl that answers from $dir/api.
dir=$(mktemp -d) && trap 'rm -rf "$dir"' EXIT
token=$dir/home/.claude/secrets/forgejo-agent-pat
mkdir -p "$(dirname "$token")" "$dir/bin" "$dir/api"
echo test-token >"$token"
# The fake tracker. A request's answer is api/<the URL past the repo, with / ? & = as _>. With no such file, a listing of
# issues under a label answers every issue on its first page, as the real tracker does for a label it lacks, and anything
# else answers an empty listing. A request named in api/fail fails. Each request's URL and arguments are logged.
cat >"$dir/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_API/../args"
config=$( [[ " $* " == *" -K - "* ]] && cat )
[[ $config == 'header = "Authorization: token test-token"' ]] || exit 22
name=$(sed 's|^https://t/||; s|[/?&=]|_|g' <<<"${!#}")
echo "$name" >>"$FAKE_API/../requests"
! grep -qx "$name" "$FAKE_API/fail" 2>/dev/null || exit 22
if [[ -f $FAKE_API/$name ]]; then cat "$FAKE_API/$name"
elif [[ $name == issues_state_open_type_issues_labels_*_page_1 ]]; then cat "$FAKE_API/every-issue"
else echo '[]'; fi
EOF
chmod +x "$dir/bin/curl"
api() { cat >"$dir/api/$1"; }
