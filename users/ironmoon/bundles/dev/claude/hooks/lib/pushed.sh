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

# Of the refs a push updates (pre-push lines on stdin), each main or master it deletes or moves to a commit its old tip
# isn't an ancestor of, as `<ref> <old> <new>`: a feature branch of ours is rewritten and force-pushed with a lease,
# main and master never.
rewritten_main() {
  local local_sha remote_ref remote_sha zero=0000000000000000000000000000000000000000
  while read -r _ local_sha remote_ref remote_sha; do
    [[ $remote_ref =~ ^refs/heads/(main|master)$ && $remote_sha != "$zero" ]] || continue
    [[ $local_sha != "$zero" ]] && git merge-base --is-ancestor "$remote_sha" "$local_sha" 2>/dev/null ||
      echo "$remote_ref $remote_sha $local_sha"
  done
}
