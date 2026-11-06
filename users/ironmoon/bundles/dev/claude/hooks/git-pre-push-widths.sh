#!/usr/bin/env bash
# git pre-push hook, set for every repo by the claude bundle's `hook.commit-width` git config, beside a repo's own: in a
# Claude Code session (CLAUDECODE set, even empty, as the bundle's jj config is scoped) it refuses a push whose new commits break the
# commit width rule (lib/widths.sh). Owen's own pushes pass untouched.

[[ -v CLAUDECODE ]] || exit 0
set -uo pipefail
. "${BASH_SOURCE[0]%/*}/lib/widths.sh"
problems=$(too_wide) || { echo "commit-width: couldn't read the pushed commits, so nothing was pushed" >&2; exit 1; }
[[ -n $problems ]] || exit 0
short() { git rev-parse --short "$1"; }
too_wide_reason "$problems" short "Reword each (\`git commit --amend\` for the tip) and push again." >&2
exit 1
