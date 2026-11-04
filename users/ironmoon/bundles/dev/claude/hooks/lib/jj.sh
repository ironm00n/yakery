# jj repos as the guards and jj-push.sh see them.

# The jj workspace dir $1 lies in: the nearest dir upward with a .jj.
repo_of() {
  local d=$1
  until [[ -d $d/.jj ]]; do
    [[ $d == / ]] && return 1
    d=${d%/*}
    d=${d:-/}
  done
  echo "$d"
}

# Sets `genv`, the environment git needs for workspace $1: none for a git worktree, which jj's
# workspaces are when colocated, else jj's git store as GIT_DIR.
git_env() {
  genv=()
  git -C "$1" rev-parse --git-dir >/dev/null 2>&1 && return
  local store
  store=$(jj --ignore-working-copy -R "$1" git root 2>/dev/null) || return
  genv=("GIT_DIR=$store" "GIT_WORK_TREE=$1")
}

# The pre-push hook of workspace $1: the one `git push` would run, else the one the repo ships in
# .githooks for `git config core.hooksPath .githooks` to enable; nothing if neither is executable.
pre_push_hook() {
  local root=$1 dir h
  git_env "$root" || return 0
  dir=$(env "${genv[@]}" git -C "$root" rev-parse --git-path hooks 2>/dev/null) || return 0
  [[ $dir == /* ]] || dir=$root/$dir
  for h in "$dir/pre-push" "$root/.githooks/pre-push"; do
    [[ -x $h ]] && { echo "$h"; break; }
  done
  return 0
}
