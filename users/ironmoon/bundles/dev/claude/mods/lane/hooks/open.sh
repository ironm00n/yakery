#!/usr/bin/env bash
# usage: open.sh REPO NUMBER...
# Those of the issues NUMBER... of the forge's repository REPO that are still open, as one JSON array, each with its
# title, how many comments it has, and the text of its "Closes with:" line (null when it has none).
set -euo pipefail
shopt -s inherit_errexit
repo=${1:?usage: open.sh REPO NUMBER...}
shift
source "$(dirname "$(realpath "$0")")/tracker.sh"
for number in "$@"; do get "issues/$number"; done |
	jq -s '[.[] | select(.state == "open") | {number, title, comments, closes: ([.body // "" | capture("Closes with:[ \\t]*(?<c>[^\\n]*)").c] | first)}]'
