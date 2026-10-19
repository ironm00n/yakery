{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.stalwart.appPasswords;
in
{
  options.services.stalwart.appPasswords = lib.mkOption {
    type = lib.types.attrsOf (
      lib.types.submodule (
        { name, ... }:
        {
          options = {
            file = lib.mkOption {
              type = lib.types.path;
              default = "${config.services.stalwart.dataDir}/app-password-${name}";
              defaultText = lib.literalExpression ''"''${config.services.stalwart.dataDir}/app-password-<address>"'';
              description = ''
                Where the app password is kept, readable by Stalwart's user only. When it is
                missing, applying the plan revokes the account's app passwords and creates one
                there. To replace it, delete it and `.plan.ndjson` beside it, then restart
                Stalwart: the missing link makes it apply the plan again.
              '';
            };

            allowedIps = lib.mkOption {
              type = lib.types.nonEmptyListOf lib.types.str;
              example = [
                "127.0.0.1/32"
                "::1/128"
              ];
              description = ''
                Networks every app password of the account may be used from, set again each
                time the plan is applied, which a change here also causes.
              '';
            };

            description = lib.mkOption {
              type = lib.types.str;
              default = name;
              description = "Description of the app password it creates.";
            };
          };
        }
      )
    );
    default = { };
    description = ''
      Accounts, by address, whose app password is made on the host when the plan is applied, for
      a program that signs in as one. Any entry also makes Stalwart's own OAuth refuse clients
      that aren't registered, and registration without its permission.
    '';
  };

  config = lib.mkIf (cfg != { }) {
    services.stalwart.plan.afterApply = lib.getExe (
      pkgs.writeShellApplication {
        name = "stalwart-app-passwords";
        runtimeInputs = [
          pkgs.curl
          pkgs.jq
        ];
        # jq programs, whose $variables are jq's own, pass through pick()
        excludeShellChecks = [ "SC2016" ];
        text = ''
          # base64 on a pipe: the password reaches neither argv nor curl's config quoting
          auth=$(printf '%s:%s' "$STALWART_USER" "$STALWART_PASSWORD" | base64 --wrap=0)
          admin() {
            # under the plan step's start timeout, which would leave Stalwart in recovery mode
            curl --fail --silent --show-error --max-time 30 \
              --config <(printf 'header = "Authorization: Basic %s"\n' "$auth") "$@"
          }
          jmap() {
            admin --header 'Content-Type: application/json' --data-binary @- "$STALWART_URL/jmap/" \
              < <(jq --null-input --compact-output \
                '{using: ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"], methodCalls: $ARGS.positional}' \
                --jsonargs "$@")
          }

          # a step that gets an unexpected response logs it, secrets removed, before failing
          pick() {
            local response
            response=$(cat)
            jq --exit-status "$@" <<<"$response" && return
            echo "unexpected response: $(jq --compact-output 'walk(if type == "object" then del(.secret) else . end)' <<<"$response")" >&2
            return 1
          }

          admin_id=$(admin "$STALWART_URL/jmap/session" | pick --raw-output '.primaryAccounts["urn:stalwart:jmap"]')

          # An unregistered OAuth client could trade an app password for tokens that ignore
          # its allowedIps. Updates must resend the provider's secrets, so they rotate too.
          jmap "$(
            OAUTH_KEY=$(head -c 48 /dev/urandom | base64 --wrap=0) \
            SIGNING_KEY=$(head -c 48 /dev/urandom | base64 --wrap=0) \
            jq --null-input --arg a "$admin_id" '["x:OidcProvider/set", {accountId: $a, update: {singleton: {
              requireClientRegistration: true,
              anonymousClientRegistration: false,
              encryptionKey: {"@type": "Value", secret: $ENV.OAUTH_KEY},
              signatureAlgorithm: "hs256",
              signatureKey: {"@type": "Text", secret: $ENV.SIGNING_KEY}
            }}}, "o"]'
          )" | pick '.methodResponses[0][1].updated | has("singleton")' >/dev/null

          account_ids=$(
            jmap \
              "$(jq --null-input --arg a "$admin_id" '["x:Account/query", {accountId: $a}, "q"]')" \
              "$(jq --null-input --arg a "$admin_id" \
                '["x:Account/get", {accountId: $a, "#ids": {resultOf: "q", name: "x:Account/query", path: "/ids"}, properties: ["emailAddress"]}, "g"]')" \
              | pick --compact-output '.methodResponses[1][1].list | map({key: .emailAddress, value: .id}) | from_entries'
          )

          app_password() {
            local address=$1 file=$2 allowed_ips=$3 description=$4 account_id old secret
            account_id=$(pick --raw-output --arg address "$address" '.[$address]' <<<"$account_ids")
            old=$(
              jmap "$(jq --null-input --arg a "$account_id" '["x:AppPassword/get", {accountId: $a, ids: null, properties: ["id"]}, "g"]')" \
                | pick --compact-output '[.methodResponses[0][1].list[].id]'
            )

            if [ -s "$file" ]; then
              jmap "$(jq --null-input --arg a "$account_id" --argjson old "$old" --argjson ips "$allowed_ips" \
                '["x:AppPassword/set", {accountId: $a, update: ($old | map({key: ., value: {allowedIps: $ips}}) | from_entries)}, "s"]')" \
                | pick '.methodResponses[0] | .[0] == "x:AppPassword/set" and (.[1].notUpdated // {}) == {}' >/dev/null
              return
            fi

            secret=$(
              jmap "$(jq --null-input --arg a "$account_id" --argjson old "$old" --argjson ips "$allowed_ips" --arg description "$description" \
                '["x:AppPassword/set", {accountId: $a, destroy: $old, create: {p: {
                  description: $description,
                  allowedIps: $ips,
                  permissions: {"@type": "Inherit"}
                }}}, "s"]')" \
                | pick --raw-output '.methodResponses[0][1] | select((.notDestroyed // {}) == {}) | .created.p.secret'
            )
            (umask 077 && printf '%s\n' "$secret" > "$file.new")
            mv "$file.new" "$file"
          }

          # a subshell in the background still stops at its first error, and stops no other account
          failed=0
          ${lib.concatMapStrings (
            { name, value }:
            "( app_password ${
              lib.escapeShellArgs [
                name
                (toString value.file)
                (builtins.toJSON (lib.genAttrs value.allowedIps (_: true)))
                value.description
              ]
            } ) & wait \"$!\" || failed=1\n"
          ) (lib.attrsToList cfg)}
          exit "$failed"
        '';
      }
    );
  };
}
