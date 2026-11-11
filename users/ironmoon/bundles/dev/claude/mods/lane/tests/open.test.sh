#!/usr/bin/env bash
# usage: bash tests/open.test.sh — hooks/open.sh against a fake tracker
set -euo pipefail
open=$(realpath "$(dirname "$0")/../hooks/open.sh")
source "$(dirname "$0")/fake-tracker.sh"
fail() { printf 'FAIL %s\n  want %s\n  got  %s\n' "$1" "$2" "$3"; exit 1; }
issue() { jq -nc --argjson n "$1" --arg state "$2" --arg body "$3" '{number: $n, title: "t\($n)", state: $state, comments: 2, body: $body}'; }
issue 42 open $'Hold the count.\n\nCloses with: the user has asked for the count and been given it.\n' | api issues_42
issue 43 closed 'Closes with: done' | api issues_43
issue 44 open 'No line here.' | api issues_44
run() { HOME=$dir/home PATH=$dir/bin:$PATH FAKE_API=$dir/api LANE_TRACKER=https://t bash "$open" agents/work "$@"; }

want='[{"number":42,"title":"t42","comments":2,"closes":"the user has asked for the count and been given it."},{"number":44,"title":"t44","comments":2,"closes":null}]'
got=$(run 42 43 44 | jq -c .)
[[ $got == "$want" ]] || fail 'the open ones of the issues named, each with its comments and what closes it' "$want" "$got"
echo 'ok: the issues still open, with their "Closes with" lines'
[[ $(run | jq -c .) == '[]' ]] || fail 'none named, none listed' '[]' "$(run)"
echo issues_42 >"$dir/api/fail"
if run 42 44 >/dev/null 2>&1; then fail 'a tracker that fails fails the run' failed ran; fi
echo 'ok: a tracker that fails fails the run'
