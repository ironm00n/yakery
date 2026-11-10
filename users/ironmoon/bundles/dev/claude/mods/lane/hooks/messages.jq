# What Owen and other sessions sent this session (`jq -cnR --argjson caps '{"owen": n, "peers": n, "message": n}'`): its
# own messages and those of peer sessions, not its subagents' reports. The transcript's live branch is walked by parent
# links across every compaction, so a rewound message is left out; where the links loop or name a row that isn't there,
# the rows before that point are taken in file order instead. Only what the kit's sections could show comes out, since a
# long-lived session was sent megabytes the caps would drop.
def text: if type == "string" then . else map(if .type == "text" then .text else "[\(.type)]" end) | join("\n") end;
# The newest messages a section of $cap characters could show, oldest first, each cut to $caps.message with the cut
# counted in `more`, and how many older ones are left out. Each costs at least one, so empty ones are bounded too.
def newest($cap):
  ([foreach (reverse[] | [(.text | length), $caps.message] | min + 1) as $cost (0; . + $cost; select(. <= $cap))] | length) as $k
  | {kept: (.[length - $k:] | map(.more = ([(.text | length) - $caps.message, 0] | max) | .text |= .[:$caps.message])),
     older: (length - $k)};
def said($origin; $content):
  if $origin.kind == "human" then {from: "Owen", text: ($content | text)}
  elif $origin.kind == "peer" and ($origin.handback | not) and $origin.senderTaskId == null then
    {from: ($origin.name // $origin.from), text: ($origin.body // ($content | text))}
  else empty end;
[inputs | fromjson? | select(.uuid != null and (.isSidechain | not))
  | {uuid, up: (.parentUuid // .logicalParentUuid),
     said: ([.timestamp as $at
       | if .type == "user" then said(.origin // {}; .message.content)
         elif .attachment.type == "queued_command" then said(.attachment.origin // {}; .attachment.prompt)
         else empty end
       | {at: $at} + .] | first)}] as $rows
| (reduce range($rows | length) as $i ({}; .[$rows[$i].uuid] = $i)) as $index
| {i: (($rows | length) - 1), live: {}, cut: -1}
| until(.i < 0;
    $rows[.i] as $row
    | .live[$row.uuid] = true
    | if $row.up == null then .i = -1
      elif $index[$row.up] == null or .live[$row.up] then .cut = .i | .i = -1
      else .i = $index[$row.up] end)
| . as $walk
| [range($rows | length) as $i | $rows[$i] | select($walk.live[.uuid] or $i < $walk.cut) | .said | values]
| {owen: (map(select(.from == "Owen")) | newest($caps.owen)), peers: (map(select(.from != "Owen")) | newest($caps.peers))}
