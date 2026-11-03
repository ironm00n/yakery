# A command for services.stalwart.plan.afterApply, running `text` as the recovery admin with these helpers.
{ lib, pkgs }:
name: text:
lib.getExe (
  pkgs.writeShellApplication {
    inherit name;
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
    ];
    excludeShellChecks = [
      # jq programs, whose $variables are jq's own, pass through pick()
      "SC2016"
      # a function `each` runs is called by its name, as an argument
      "SC2329"
    ];
    text = ''
      # base64 on a pipe: the password reaches neither argv nor curl's config quoting
      auth=$(printf '%s:%s' "$STALWART_USER" "$STALWART_PASSWORD" | base64 --wrap=0)
      request() {
        # under the plan step's start timeout, which would leave Stalwart in recovery mode
        curl --silent --show-error --max-time 30 \
          --config <(printf 'header = "Authorization: Basic %s"\n' "$auth") "$@"
      }
      admin() {
        request --fail "$@"
      }
      jmap() {
        admin --header 'Content-Type: application/json' --data-binary @- "$STALWART_URL/jmap/" \
          < <(jq --null-input --compact-output \
            '{using: ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:calendars", "urn:stalwart:jmap"], methodCalls: $ARGS.positional}' \
            --jsonargs "$@")
      }

      pick() {
        local response
        response=$(cat)
        jq --exit-status "$@" <<<"$response" && return
        echo "unexpected response: $(jq --compact-output 'walk(if type == "object" then del(.secret) else . end)' <<<"$response")" >&2
        return 1
      }

      admin_id=$(admin "$STALWART_URL/jmap/session" | pick --raw-output '.primaryAccounts["urn:stalwart:jmap"]')
      account_ids=$(
        jmap \
          "$(jq --null-input --arg a "$admin_id" '["x:Account/query", {accountId: $a}, "q"]')" \
          "$(jq --null-input --arg a "$admin_id" \
            '["x:Account/get", {accountId: $a, "#ids": {resultOf: "q", name: "x:Account/query", path: "/ids"}, properties: ["emailAddress"]}, "g"]')" \
          | pick --compact-output '.methodResponses[1][1].list | map({key: .emailAddress, value: .id}) | from_entries'
      )

      failed=0
      each() {
        # in the background, a subshell still stops at its first error
        ( "$@" ) & wait "$!" || failed=1
      }

      ${text}
      exit "$failed"
    '';
  }
)
