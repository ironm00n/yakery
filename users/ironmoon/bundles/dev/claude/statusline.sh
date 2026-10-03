#!/usr/bin/env bash
# Claude Code status line. Receives the session JSON on stdin; first line of
# stdout is rendered. Schema: https://code.claude.com/docs/en/statusline
exec jq -rj '
  def esc: [27] | implode;
  def sgr($c; $s): "\(esc)[\($c)m\($s)\(esc)[0m";

  def tilde: if startswith(env.HOME) then "~" + .[(env.HOME | length):] else . end;

  def relative($root): if startswith($root + "/") then "./" + .[($root | length) + 1:] else tilde end;

  def dirs: .project_dir as $p
    | [sgr(34; $p | tilde), (.current_dir | select(. != $p) | relative($p) | sgr(36; .))]
    | join(" ");

  # shared scale: every percentage shown is "consumed", so hotter is worse
  def heat: if . >= 90 then 31 elif . >= 70 then 33 else 32 end;

  def gauge($label): if . == null then empty else sgr(heat; "\($label) \(floor)%") end;

  def si: if . >= 1e6 then "\(. / 1e5 | floor / 10)m"
    elif . >= 1e3 then "\(. / 1e3 | floor)k" else "\(.)" end;

  def ctx: select(.used_percentage)
    | (.used_percentage | heat) as $h
    | "\(sgr($h; .total_input_tokens | si))/\(.context_window_size | si) (\(sgr($h; "\(.used_percentage | floor)%")))";

  def countdown: ([. - now, 0] | max) / 60 | ceil
    | [[(. / 1440 | floor), "d"], [(. / 60 | floor) % 24, "h"], [. % 60, "m"]]
    | until(length == 1 or .[0][0] > 0; .[1:]) | .[:2] | map("\(.[0])\(.[1])") | add;

  def window($label): select(.)
    | [(.used_percentage | gauge($label)), (.resets_at | select(.) | countdown | sgr(90; .))]
    | join(" ");

  def cache: select(.)
    | if .warm and .expires_at then sgr(32; "cache") + " " + (.expires_at | countdown | sgr(90; .))
      else sgr(33; "cache cold") end;

  [ (.workspace | dirs),
    ([.model.display_name, (.effort.level | select(.) | "(\(.))")] | join(" ") | sgr(35; .)),
    (.context_window        | ctx),
    (.prompt_cache          | cache),
    (.rate_limits.five_hour | window("5h")),
    (.rate_limits.seven_day | window("7d")),
    (.version               | select(.) | "v\(.)")
  ] | join(sgr(90; " · "))
'
