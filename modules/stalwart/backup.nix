{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.stalwart.backup;
in
{
  options.services.stalwart.backup = {
    enable = lib.mkEnableOption ''
      a dump command that refreshes one Vandelay archive per user and group
      account, named by account id with `accounts.json` mapping ids to
      addresses, and writes all of them to stdout as a tar stream. An account
      whose archive fails keeps its previous one in the stream, and the command
      then exits non-zero. It also makes Stalwart's own OAuth refuse clients
      that aren't registered, and registration without its permission
    '';

    package = lib.mkOption {
      type = lib.types.package;
      description = "Vandelay.";
    };

    account = lib.mkOption {
      type = lib.types.str;
      example = "backup@example.org";
      description = ''
        Address of the account the archives are taken through. Give it a
        `Custom` role with no role ids and
        {option}`services.stalwart.backup.permissions`, nothing more.
      '';
    };

    permissions = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      readOnly = true;
      description = ''
        Everything {option}`services.stalwart.backup.account` needs: listing
        accounts, impersonating them, and the JMAP methods that only read.
        Impersonation applies the account's own permissions to every account it
        reads, so any write or credential permission here would reach them all.
      '';
    };

    appPasswordFile = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/stalwart/backup-app-password";
      description = ''
        File holding an app password of {option}`services.stalwart.backup.account`.
        When it is missing, applying the plan revokes the account's app
        passwords and creates one there, allowed from loopback only. To replace
        it, delete it and `.plan.ndjson` beside it, then restart Stalwart: the
        missing link makes it apply the plan again.
      '';
    };

    url = lib.mkOption {
      type = lib.types.str;
      default = "http://127.0.0.1:8080";
      description = "Stalwart's HTTP listener, over loopback.";
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/stalwart-backup";
      description = ''
        Where the archives persist between runs, so each run only fetches what
        changed. An archive is left in place after its account is deleted.
      '';
    };

    dumpCommand = lib.mkOption {
      type = lib.types.path;
      readOnly = true;
      description = ''
        The dump command, run as root. Pass it to a backup job together with
        {option}`services.stalwart.backup.stateDir` as a writable path.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    services.stalwart.backup.permissions = [
      "authenticate"
      "impersonate"
      "sysAccountQuery"
      "sysAccountGet"
    ]
    ++ lib.filter (
      permission:
      lib.hasPrefix "jmap" permission
      && lib.any (read: lib.hasSuffix read permission) [
        "Get"
        "Query"
        "Changes"
        "Lookup"
      ]
    ) (lib.attrValues config.services.stalwart.package.permissions);

    services.stalwart.plan.afterApply = lib.getExe (
      pkgs.writeShellApplication {
        name = "stalwart-backup-app-password";
        runtimeInputs = [
          pkgs.curl
          pkgs.jq
        ];
        # jq programs, whose $variables are jq's own, pass through pick()
        excludeShellChecks = [ "SC2016" ];
        text = ''
          password_file=${lib.escapeShellArg (toString cfg.appPasswordFile)}

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

          # An unregistered OAuth client could trade the app password for tokens that ignore
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

          [ -s "$password_file" ] && exit 0

          account_id=$(
            jmap \
              "$(jq --null-input --arg a "$admin_id" '["x:Account/query", {accountId: $a}, "q"]')" \
              "$(jq --null-input --arg a "$admin_id" \
                '["x:Account/get", {accountId: $a, "#ids": {resultOf: "q", name: "x:Account/query", path: "/ids"}, properties: ["emailAddress"]}, "g"]')" \
              | pick --raw-output --arg address ${lib.escapeShellArg cfg.account} \
                '.methodResponses[1][1].list[] | select(.emailAddress == $address) | .id'
          )
          old=$(
            jmap "$(jq --null-input --arg a "$account_id" '["x:AppPassword/get", {accountId: $a, ids: null, properties: ["id"]}, "g"]')" \
              | pick --compact-output '[.methodResponses[0][1].list[].id]'
          )
          secret=$(
            jmap "$(jq --null-input --arg a "$account_id" --argjson old "$old" '["x:AppPassword/set", {accountId: $a, destroy: $old, create: {p: {
              description: "backup dump",
              allowedIps: {"127.0.0.1/32": true, "::1/128": true},
              permissions: {"@type": "Inherit"}
            }}}, "s"]')" \
              | pick --raw-output '.methodResponses[0][1] | select((.notDestroyed // {}) == {}) | .created.p.secret'
          )
          (umask 077 && printf '%s\n' "$secret" > "$password_file.new")
          mv "$password_file.new" "$password_file"
        '';
      }
    );

    services.stalwart.backup.dumpCommand =
      pkgs.writers.writePython3 "stalwart-backup-dump"
        {
          flakeIgnore = [ "E501" ];
        }
        ''
          import json
          import os
          import subprocess
          import sys
          import tarfile
          import urllib.error
          import urllib.request
          from base64 import b64encode
          from pathlib import Path

          URL = ${builtins.toJSON cfg.url}
          ACCOUNT = ${builtins.toJSON cfg.account}
          STATE = Path(${builtins.toJSON (toString cfg.stateDir)})
          PASSWORD_FILE = Path(${builtins.toJSON (toString cfg.appPasswordFile)})
          REPAIR = (
              f"delete {PASSWORD_FILE} and .plan.ndjson beside it, then restart Stalwart"
              " so it applies the plan and makes a new one"
          )

          password = PASSWORD_FILE.read_text().strip() if PASSWORD_FILE.exists() else ""
          if not password:
              sys.exit(f"no app password in {PASSWORD_FILE}: {REPAIR}")
          auth = "Basic " + b64encode(f"{ACCOUNT}:{password}".encode()).decode()


          def request(path, body=None):
              req = urllib.request.Request(
                  URL + path,
                  data=None if body is None else json.dumps(body).encode(),
                  headers={"Authorization": auth, "Content-Type": "application/json"},
              )
              with urllib.request.urlopen(req) as response:
                  return json.load(response)


          try:
              own_id = request("/jmap/session")["primaryAccounts"]["urn:stalwart:jmap"]
          except urllib.error.HTTPError as error:
              if error.code == 401:
                  sys.exit(f"{ACCOUNT} was refused its app password: {REPAIR}")
              raise
          responses = request("/jmap/", {
              "using": ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
              "methodCalls": [
                  ["x:Account/query", {"accountId": own_id}, "q"],
                  ["x:Account/get", {
                      "accountId": own_id,
                      "#ids": {"resultOf": "q", "name": "x:Account/query", "path": "/ids"},
                      "properties": ["@type", "emailAddress"],
                  }, "g"],
              ],
          })["methodResponses"]
          name, accounts, _ = responses[1]
          if name != "x:Account/get":
              sys.exit(f"listing accounts failed: {responses}")

          # Archives are named by account id: an address can come back on a new
          # account, whose archive Vandelay would refuse to refresh.
          manifest_file = STATE / "accounts.json"
          manifest = json.loads(manifest_file.read_text()) if manifest_file.exists() else {}
          env = os.environ | {"VANDELAY_PASSWORD": password}
          failed = []
          for account in accounts["list"]:
              if account["@type"] not in ("User", "Group") or account["emailAddress"] == ACCOUNT:
                  continue
              manifest[account["id"]] = {"type": account["@type"], "address": account["emailAddress"]}
              archived = subprocess.run(
                  [
                      ${builtins.toJSON (lib.getExe cfg.package)},
                      "import", "jmap",
                      "--url", URL,
                      "--use-url-origin",
                      "--auth-basic", ACCOUNT,
                      "--account-id", account["id"],
                      str(STATE / f"{account['id']}.sqlite"),
                  ],
                  env=env,
                  stdout=sys.stderr,
              )
              if archived.returncode != 0:
                  failed.append(account["emailAddress"])
          # a manifest cut short would stop every later run at json.loads
          new_manifest = manifest_file.with_suffix(".new")
          new_manifest.write_text(json.dumps(manifest, indent=2) + "\n")
          new_manifest.replace(manifest_file)

          with tarfile.open(fileobj=sys.stdout.buffer, mode="w|") as tar:
              tar.add(STATE, arcname=".")
          if failed:
              sys.exit("archives not fully refreshed: " + ", ".join(failed))
        '';

    systemd.tmpfiles.settings.stalwart-backup.${cfg.stateDir}.d = {
      mode = "0700";
      user = "root";
      group = "root";
    };
  };
}
