#!/usr/bin/env bash
# Tells the model which model, effort and harness version it runs, for commit trailers: at session
# and subagent start, after compaction, and whenever one of them changes. SessionStart input carries
# neither model nor effort (2.1.296): only tool hooks see `effort.level`, and the model is read once
# from the transcript, which can lag the first PreToolUse (so PostToolUse runs this too), then
# follows PostModelSwitch.

set -uo pipefail

payload=$(cat)
command -v jq >/dev/null 2>&1 || exit 0

field() { jq -r "$1 // empty" <<<"$payload"; }

event=$(field .hook_event_name)
session=$(field .session_id)
agent=$(field .agent_id)
transcript=$(field .transcript_path)
[[ -n "$session" ]] || exit 0

state_dir="${XDG_RUNTIME_DIR:-/tmp}/claude-session-facts/$session"
mkdir -p "$state_dir" || exit 0

emit() {
  jq -cn --arg e "$event" --arg c "Provenance facts: $*" \
    '{hookSpecificOutput: {hookEventName: $e, additionalContext: $c}}'
  exit 0
}

harness_version() {
  local pid=$PPID exe
  while ((pid > 1)); do
    exe=$(readlink "/proc/$pid/exe" 2>/dev/null) || return
    if [[ "$exe" =~ -claude-code-([0-9][^/]*)/ ]]; then
      echo "${BASH_REMATCH[1]}"
      return
    fi
    pid=$(awk '/^PPid:/ { print $2 }' "/proc/$pid/status" 2>/dev/null) || return
  done
}

# Embedded agent progress can nest `"type":"assistant"` inside another entry, so candidates are
# re-checked as whole entries.
transcript_model() {
  [[ -r "$1" ]] || return
  tac "$1" |
    awk -v a='"type":"assistant"' -v m='"model":"claude-' \
      'index($0, a) && index($0, m) { print; if (++n == 5) exit }' |
    jq -r 'select(.type == "assistant") | .message.model | select(startswith("claude-"))' |
    head -n 1
}

# State is `model|effort|version`; `|` keeps empty fields in place.
read_state() { [[ -f "$state_dir/$1" ]] && cat "$state_dir/$1"; }

root_of() {
  local model effort version
  IFS='|' read -r model effort version <<<"$1"
  echo "$model${effort:+:$effort} claude-code${version:+:$version}"
}

case "$event" in
SessionStart)
  rm -f "$state_dir/main"
  version=$(harness_version)
  model=$(field .model)
  model=${model:-$(transcript_model "$transcript")}
  emit "harness claude-code${version:+ $version}${model:+, model $model};" \
    "the trailer root follows with the first tool call that knows the effort."
  ;;
PostModelSwitch)
  model=$(field .to_model)
  [[ -n "$model" ]] || exit 0
  IFS='|' read -r _ effort version <<<"$(read_state main)"
  facts="$model|${effort:-}|${version:-$(harness_version)}"
  echo "$facts" >"$state_dir/main"
  emit "model switched from $(field .from_model) to $model; trailer root \`$(root_of "$facts")\`."
  ;;
SubagentStart)
  own="your own model and effort follow with your first tool call."
  main=$(read_state main) || emit "$own"
  emit "the session that spawned you has trailer root \`$(root_of "$main")\`; $own"
  ;;
PreToolUse | PostToolUse)
  key=${agent:-main}
  previous=$(read_state "$key") || previous=""
  IFS='|' read -r model previous_effort version <<<"$previous"
  if [[ -z "$model" ]]; then
    if [[ -n "$agent" ]]; then
      model=$(transcript_model "${transcript%.jsonl}/subagents/agent-$agent.jsonl")
    else
      model=$(transcript_model "$transcript")
    fi
  fi
  effort=$(field .effort.level)
  [[ -n "$model" && -n "$effort" ]] || exit 0
  facts="$model|$effort|${version:-$(harness_version)}"
  [[ "$facts" == "$previous" ]] && exit 0
  echo "$facts" >"$state_dir/$key"
  if [[ -n "$agent" ]]; then
    main=$(read_state main) || main=""
    emit "you run \`$model:$effort\`${main:+, under the trailer root \`$(root_of "$main")\`}."
  fi
  [[ -z "$previous_effort" ]] && emit "trailer root \`$(root_of "$facts")\`."
  emit "trailer root is now \`$(root_of "$facts")\`, was \`$(root_of "$previous")\`."
  ;;
esac

exit 0
