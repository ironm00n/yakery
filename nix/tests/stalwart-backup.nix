{ pkgs }:
let
  inherit (import (pkgs.path + "/nixos/tests/ssh-keys.nix") pkgs)
    snakeOilEd25519PrivateKey
    snakeOilEd25519PublicKey
    ;

  borg = "${pkgs.borgbackup}/bin/borg";
  box = import ./borg-box.nix { inherit pkgs; };
  forcedCommand = "${borg} serve --append-only --restrict-to-repository /home/u/stalwart";

  stalwart = import ./stalwart-server.nix { inherit pkgs; };
  inherit (stalwart) domain account;
  vandelay = pkgs.callPackage ../../packages/vandelay { };
in
pkgs.testers.runNixOSTest {
  name = "stalwart-backup";

  defaults.virtualisation.graphics = false;

  nodes = {
    box = box.node;

    server =
      { config, ... }:
      {
        imports = [
          stalwart.node
          ../../bundles/backup.nix
          # sops-nix stays out of tests; the services here name plain files instead
          { options.sops.secrets = pkgs.lib.mkOption { type = pkgs.lib.types.attrs; }; }
        ];
        _module.args = {
          my-lib = import ../../lib { inherit (pkgs) lib; };
          inputs = { };
        };

        services.stalwart.plan.sequence = [
          (account "alice" { })
          (account "backup" {
            roles = {
              "@type" = "Custom";
              roleIds = { };
            };
            permissions = {
              "@type" = "Merge";
              enabledPermissions = pkgs.lib.genAttrs config.services.stalwart.backup.permissions (_: true);
            };
          })
          {
            "@type" = "upsert";
            object = "Account";
            matchOn = [
              "name"
              "domainId"
            ];
            value.team = {
              "@type" = "Group";
              name = "team";
              domainId = "#domain";
              permissions."@type" = "Inherit";
              roles = {
                "@type" = "Custom";
                roleIds = { };
              };
            };
          }
        ];

        services.stalwart.backup = {
          enable = true;
          package = vandelay;
          account = "backup@${domain}";
        };

        environment.etc."backup/passphrase".text = "correct horse battery staple";
        environment.etc."backup/ssh-key" = {
          source = snakeOilEd25519PrivateKey;
          mode = "0600";
        };
        environment.systemPackages = [ pkgs.sqlite ];

        bundles.backup = {
          target = {
            host = "box";
            user = "u";
            hostPublicKey = box.hostKey.public;
            remotePath = borg;
          };
          services.stalwart = {
            secretsFile = null;
            passphraseFile = "/etc/backup/passphrase";
            sshKeyFile = "/etc/backup/ssh-key";
            inherit (config.services.stalwart.backup) dumpCommand;
            readWritePaths = [ config.services.stalwart.backup.stateDir ];
          };
        };
      };
  };

  testScript =
    { nodes, ... }:
    stalwart.python
    + ''
      start_all()
      box.wait_for_unit("sshd.service")
      box.succeed(
          "install -d -m 700 -o u -g users /home/u/.ssh",
          "echo 'command=\"${forcedCommand}\",restrict ${snakeOilEd25519PublicKey}' > /home/u/.ssh/authorized_keys",
          "chown u:users /home/u/.ssh/authorized_keys",
          "chmod 600 /home/u/.ssh/authorized_keys",
      )

      wait_for_stalwart()

      password_file = "${nodes.server.services.stalwart.backup.appPasswordFile}"
      with subtest("applying the plan made the backup account's app password, readable by stalwart only"):
          assert server.succeed(f"stat -c '%a %U' {password_file}").strip() == "600 stalwart"
      backup_secret = server.succeed(f"cat {password_file}").strip()
      as_backup = basic("backup@${domain}", backup_secret)

      with subtest("the backup password can't be traded for OAuth tokens or register a client"):
          out = server.succeed(
              "curl -s -w '\\n%{http_code}' -H 'Content-Type: application/json' http://127.0.0.1:8080/api/auth --data "
              + shlex.quote(json.dumps({
                  "type": "authCode",
                  "accountName": "backup@${domain}",
                  "accountSecret": backup_secret,
                  "clientId": "trade",
                  "redirectUri": "https://trade.test/cb",
              }))
          )
          assert "client_code" not in out, out
          status, payload = curl(server, as_backup, "/auth/register", {"redirect_uris": ["https://trade.test/cb"]})
          assert status in (401, 403), (status, payload)

      alice = account_ids["alice"]
      inbox = call(admin, alice, "Mailbox/query", {"filter": {"role": "inbox"}})["ids"][0]
      created = call(admin, alice, "Email/set", {"create": {"m": {
          "mailboxIds": {inbox: True},
          "from": [{"email": "sender@remote.test"}],
          "subject": "kept",
          "bodyValues": {"b": {"value": "body"}},
          "textBody": [{"partId": "b", "type": "text/plain"}],
      }}})
      assert "m" in created.get("created", {}), created

      def run_job():
          server.succeed("systemctl start --wait borgbackup-job-stalwart.service")
          result = server.succeed("systemctl show -P Result borgbackup-job-stalwart.service").strip()
          assert result == "success", result
          return server.succeed("borg-job-stalwart list --short").split()

      with subtest("the backup password is refused from another node"):
          status, _ = session(box, as_backup)
          assert status == 403, status

      def as_backup_call(account_id, method, args):
          status, payload = curl(server, as_backup, "/jmap/", {
              "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:stalwart:jmap"],
              "methodCalls": [[method, {"accountId": account_id, **args}, "0"]],
          })
          assert status == 200, payload
          name, result, _ = json.loads(payload)["methodResponses"][0]
          return name, result

      with subtest("the backup account reads alice's mail but can't change it or mint her credentials"):
          name, result = as_backup_call(alice, "Email/query", {})
          assert name == "Email/query" and result["ids"], result
          name, result = as_backup_call(alice, "Email/set", {"destroy": [created["created"]["m"]["id"]]})
          assert name == "error" or result.get("notDestroyed"), result
          name, result = as_backup_call(alice, "x:AppPassword/set", {"create": {"p": {
              "description": "stolen",
              "permissions": {"@type": "Inherit"},
          }}})
          assert name == "error" or "p" in result.get("notCreated", {}), result

      with subtest("the dump archives alice's mail through the backup account"):
          server.wait_until_succeeds(
              "rm -rf /tmp/restore && mkdir /tmp/restore"
              " && systemctl start --wait borgbackup-job-stalwart.service"
              " && borg-job-stalwart extract --stdout ::$(borg-job-stalwart list --short --last 1) stalwart.dump | tar -x -C /tmp/restore"
              f" && sqlite3 /tmp/restore/{alice}.sqlite 'SELECT data FROM emails JOIN blobs ON blobs.id = emails.blob_id'"
              " | grep -q 'Subject: kept'",
              timeout=120,
          )

      with subtest("the archives are named by account id, groups included, with a manifest of addresses"):
          manifest = json.loads(server.succeed("cat /tmp/restore/accounts.json"))
          assert manifest[alice]["address"] == "alice@${domain}", manifest
          team = account_ids["team"]
          assert manifest[team] == {"type": "Group", "address": "team@${domain}"}, manifest
          server.succeed(f"test -s /tmp/restore/{team}.sqlite")

      with subtest("the backup password is still refused from another node once the dump's login is cached"):
          status, _ = session(box, as_backup)
          assert status == 403, status

      with subtest("a second run refreshes the archives it already has"):
          archives = run_job()
          assert len(archives) >= 2, archives

      with subtest("the documented repair makes a new password and revokes the old one"):
          server.succeed(f"rm {password_file} /var/lib/stalwart/.plan.ndjson && systemctl restart stalwart.service")
          wait_for_stalwart()
          renewed = basic("backup@${domain}", server.succeed(f"cat {password_file}").strip())
          assert session(server, renewed)[0] == 200
          assert session(server, as_backup)[0] == 401
          run_job()
    '';
}
