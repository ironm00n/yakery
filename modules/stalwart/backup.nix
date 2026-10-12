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
      a dump command that refreshes one Vandelay archive per user account and
      writes all of them to stdout as a tar stream
    '';

    package = lib.mkOption {
      type = lib.types.package;
      description = "Vandelay.";
    };

    account = lib.mkOption {
      type = lib.types.str;
      example = "backup@example.org";
      description = ''
        Address of the account the archives are taken through. It needs the
        `impersonate` permission to read each user's account, and
        `sysAccountQuery` and `sysAccountGet` to list them.
      '';
    };

    appPasswordFile = lib.mkOption {
      type = lib.types.path;
      description = ''
        File holding an app password of {option}`services.stalwart.backup.account`.
        Restrict its `allowedIps` to loopback: it can read and write every user's
        account.
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
          import urllib.request
          from base64 import b64encode
          from pathlib import Path

          URL = ${builtins.toJSON cfg.url}
          ACCOUNT = ${builtins.toJSON cfg.account}
          STATE = Path(${builtins.toJSON (toString cfg.stateDir)})

          password = Path(${builtins.toJSON (toString cfg.appPasswordFile)}).read_text().strip()
          auth = "Basic " + b64encode(f"{ACCOUNT}:{password}".encode()).decode()


          def request(path, body=None):
              req = urllib.request.Request(
                  URL + path,
                  data=None if body is None else json.dumps(body).encode(),
                  headers={"Authorization": auth, "Content-Type": "application/json"},
              )
              with urllib.request.urlopen(req) as response:
                  return json.load(response)


          own_id = request("/jmap/session")["primaryAccounts"]["urn:stalwart:jmap"]
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

          env = os.environ | {"VANDELAY_PASSWORD": password}
          for account in accounts["list"]:
              if account["@type"] != "User" or account["emailAddress"] == ACCOUNT:
                  continue
              subprocess.run(
                  [
                      ${builtins.toJSON (lib.getExe cfg.package)},
                      "import", "jmap",
                      "--url", URL,
                      "--use-url-origin",
                      "--auth-basic", ACCOUNT,
                      "--account-id", account["id"],
                      str(STATE / f"{account['emailAddress']}.sqlite"),
                  ],
                  env=env,
                  stdout=sys.stderr,
                  check=True,
              )

          with tarfile.open(fileobj=sys.stdout.buffer, mode="w|") as tar:
              tar.add(STATE, arcname=".")
        '';

    systemd.tmpfiles.settings.stalwart-backup.${cfg.stateDir}.d = {
      mode = "0700";
      user = "root";
      group = "root";
    };
  };
}
