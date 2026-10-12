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
  passwordFile = "/run/stalwart-backup-password";
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
            permissions = {
              "@type" = "Merge";
              enabledPermissions = {
                impersonate = true;
                sysAccountQuery = true;
                sysAccountGet = true;
              };
            };
          })
        ];

        services.stalwart.backup = {
          enable = true;
          package = vandelay;
          account = "backup@${domain}";
          appPasswordFile = passwordFile;
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

  testScript = stalwart.python + ''
    start_all()
    box.wait_for_unit("sshd.service")
    box.succeed(
        "install -d -m 700 -o u -g users /home/u/.ssh",
        "echo 'command=\"${forcedCommand}\",restrict ${snakeOilEd25519PublicKey}' > /home/u/.ssh/authorized_keys",
        "chown u:users /home/u/.ssh/authorized_keys",
        "chmod 600 /home/u/.ssh/authorized_keys",
    )

    wait_for_stalwart()
    secret = app_password("backup")
    server.succeed(f"install -m 600 /dev/stdin ${passwordFile} <<< {shlex.quote(secret)}")
    as_backup = basic("backup@${domain}", secret)

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

    with subtest("the dump archives alice's mail through the backup account"):
        server.wait_until_succeeds(
            "rm -rf /tmp/restore && mkdir /tmp/restore"
            " && systemctl start --wait borgbackup-job-stalwart.service"
            " && borg-job-stalwart extract --stdout ::$(borg-job-stalwart list --short --last 1) stalwart.dump | tar -x -C /tmp/restore"
            " && sqlite3 /tmp/restore/alice@${domain}.sqlite 'SELECT data FROM emails JOIN blobs ON blobs.id = emails.blob_id'"
            " | grep -q 'Subject: kept'",
            timeout=120,
        )

    with subtest("the backup password is still refused from another node once the dump's login is cached"):
        status, _ = session(box, as_backup)
        assert status == 403, status

    with subtest("a second run refreshes the archives it already has"):
        archives = run_job()
        assert len(archives) >= 2, archives
  '';
}
