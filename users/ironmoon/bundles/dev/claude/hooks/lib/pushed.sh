# What a push sends: the commits it would put on the remote that aren't on any remote yet, read from pre-push lines
# (`<ref> <sha> <ref> <sha>`) on stdin. Git runs as the caller's environment sets it up (GIT_DIR for a jj store).

# `git log` of those commits with the arguments given (a format), or nothing for a push that sends none.
pushed_log() {
  local local_sha remote_sha tips=() known=() zero=0000000000000000000000000000000000000000
  while read -r _ local_sha _ remote_sha; do
    [[ $local_sha == "$zero" ]] || tips+=("$local_sha")
    [[ $remote_sha == "$zero" ]] || ! git cat-file -e "$remote_sha^{commit}" 2>/dev/null || known+=("^$remote_sha")
  done
  ((${#tips[@]})) || return 0
  git log --no-show-signature "$@" "${tips[@]}" "${known[@]}" --not --remotes
}
