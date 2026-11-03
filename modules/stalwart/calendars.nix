{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.stalwart.calendars;
  inherit (lib) mkOption types;
  inherit (import ../../lib/stalwart.nix { inherit lib; }) calendarHref;
  rights = [
    "mayReadFreeBusy"
    "mayReadItems"
    "mayWriteAll"
    "mayWriteOwn"
    "mayUpdatePrivate"
    "mayRSVP"
    "mayShare"
    "mayDelete"
  ];
  calendars = lib.concatLists (
    lib.mapAttrsToList (
      holder: lib.mapAttrsToList (path: calendar: { inherit holder path calendar; })
    ) cfg
  );
in
{
  options.services.stalwart.calendars = mkOption {
    type = types.attrsOf (
      types.attrsOf (
        types.submodule (
          { name, ... }:
          {
            options = {
              name = mkOption {
                type = types.str;
                default = name;
                description = "Its display name, by which applying the plan finds it again.";
              };
              shares = mkOption {
                type = types.attrsOf (types.nonEmptyListOf (types.enum rights));
                default = { };
                description = "Every account it is shared with, by address, and what each may do; any other share is removed.";
              };
            };
          }
        )
      )
    );
    default = { };
    description = ''
      Calendars made when the plan is applied, by the address of the account that holds them and
      their CalDAV path segment there, each shared with exactly its `shares`. Removing one leaves
      it in Stalwart.
    '';
  };

  config = lib.mkIf (cfg != { }) {
    assertions = lib.mapAttrsToList (holder: held: {
      assertion = lib.allUnique (lib.mapAttrsToList (_: c: c.name) held);
      message = "services.stalwart.calendars.\"${holder}\" gives two calendars one name, by which each is found";
    }) cfg;

    services.stalwart.plan.afterApply =
      import ./admin-script.nix { inherit lib pkgs; } "stalwart-calendars"
        ''
          calendar() {
            local holder=$1 url=$STALWART_URL$2 name=$3 shares=$4 holder_id status id
            holder_id=$(pick --raw-output --arg a "$holder" '.[$a]' <<<"$account_ids")
            status=$(
              request --request MKCALENDAR --header 'Content-Type: application/xml' --output /dev/null --write-out '%{http_code}' \
                --data-binary "$(jq --null-input --raw-output --arg n "$name" '$n | @html | "<C:mkcalendar xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\"><D:set><D:prop><D:displayname>\(.)</D:displayname></D:prop></D:set></C:mkcalendar>"')" \
                "$url"
            )
            # 405: it was made by an earlier apply
            if [ "$status" != 201 ] && [ "$status" != 405 ]; then
              echo "MKCALENDAR $url answered $status" >&2
              return 1
            fi
            id=$(
              jmap "$(jq --null-input --arg a "$holder_id" '["Calendar/get", {accountId: $a, properties: ["name"]}, "c"]')" \
                | pick --raw-output --arg n "$name" '[.methodResponses[0][1].list[] | select(.name == $n) | .id] | select(length == 1) | .[0]'
            )
            jmap "$(jq --null-input --arg a "$holder_id" --arg c "$id" --argjson shares "$shares" --argjson ids "$account_ids" \
              '["Calendar/set", {accountId: $a, update: {($c): {shareWith: ($shares | with_entries(.key |= ($ids[.] // error("no account \(.)"))))}}}, "s"]')" \
              | pick --arg c "$id" '.methodResponses[0][1].updated | has($c)' >/dev/null
          }

          ${lib.concatMapStrings (
            {
              holder,
              path,
              calendar,
            }:
            "each calendar ${
              lib.escapeShellArgs [
                holder
                (calendarHref holder path)
                calendar.name
                (builtins.toJSON (lib.mapAttrs (_: lib.flip lib.genAttrs (_: true)) calendar.shares))
              ]
            }\n"
          ) calendars}
        '';
  };
}
