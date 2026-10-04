{ pkgs }:
let
  inherit (import (pkgs.path + "/nixos/tests/ssh-keys.nix") pkgs)
    snakeOilEd25519PrivateKey
    snakeOilEd25519PublicKey
    ;

  borg = "${pkgs.borgbackup}/bin/borg";
  box = import ./borg-box.nix { inherit pkgs; };
  forcedCommand = "${borg} serve --append-only --restrict-to-repository /home/u/demo";
in
pkgs.testers.runNixOSTest {
  name = "backup-bundle";

  defaults.virtualisation.graphics = false;

  nodes = {
    box = box.node;

    client =
      { config, pkgs, ... }:
      {
        imports = [
          ../../bundles/backup.nix
          # sops-nix stays out of tests; the services here name plain files instead
          { options.sops.secrets = pkgs.lib.mkOption { type = pkgs.lib.types.attrs; }; }
        ];
        _module.args = {
          my-lib = import ../../lib { inherit (pkgs) lib; };
          inputs = { };
        };

        services.postgresql = {
          enable = true;
          ensureDatabases = [ "demo" ];
        };

        environment.etc."backup/passphrase".text = "correct horse battery staple";
        environment.etc."backup/ssh-key" = {
          source = snakeOilEd25519PrivateKey;
          mode = "0600";
        };

        bundles.backup = {
          target = {
            host = "box";
            user = "u";
            hostPublicKey = box.hostKey.public;
            remotePath = borg;
          };
          services.demo = {
            secretsFile = null;
            passphraseFile = "/etc/backup/passphrase";
            sshKeyFile = "/etc/backup/ssh-key";
            dumpCommand = pkgs.writeShellScript "demo-dump" ''
              exec ${pkgs.util-linux}/bin/setpriv --reuid=postgres --regid=postgres --clear-groups \
                ${config.services.postgresql.package}/bin/pg_dump --format=custom demo
            '';
          };
        };
      };
  };

  testScript = ''
    start_all()
    box.wait_for_unit("sshd.service")
    box.succeed(
        "install -d -m 700 -o u -g users /home/u/.ssh",
        "echo 'command=\"${forcedCommand}\",restrict ${snakeOilEd25519PublicKey}' > /home/u/.ssh/authorized_keys",
        "chown u:users /home/u/.ssh/authorized_keys",
        "chmod 600 /home/u/.ssh/authorized_keys",
    )
    client.wait_for_unit("postgresql.service")
    client.wait_until_succeeds("sudo -u postgres psql demo -c 'select 1'")
    client.succeed("sudo -u postgres psql demo -c \"create table t (v text); insert into t values ('kept')\"")

    with subtest("the job initialises its repository and stores the dump"):
        client.succeed("systemctl start --wait borgbackup-job-demo.service")
        result = client.succeed("systemctl show -P Result borgbackup-job-demo.service").strip()
        assert result == "success", result
        archives = client.succeed("borg-job-demo list --short").split()
        assert len(archives) == 1, archives

    with subtest("the dump restores into a fresh database"):
        client.succeed(
            f"borg-job-demo extract --stdout ::{archives[0]} demo.dump > /tmp/demo.dump",
            "sudo -u postgres createdb restored",
            "sudo -u postgres pg_restore --dbname=restored /tmp/demo.dump",
        )
        restored = client.succeed("sudo -u postgres psql restored -tAc 'select v from t'").strip()
        assert restored == "kept", restored

    with subtest("deleting and compacting through the host's key frees nothing on the box"):
        before = set(box.succeed("find /home/u/demo/data -type f").split())
        client.succeed(f"borg-job-demo delete ::{archives[0]}")
        status, out = client.execute("borg-job-demo compact 2>&1")
        print(f"host compact: exit {status}: {out}")
        after = set(box.succeed("find /home/u/demo/data -type f").split())
        assert before <= after, before - after
        # control: the box itself, unrestricted, does find something to free
        box.succeed("runuser -u u -- ${borg} compact /home/u/demo")
        compacted = set(box.succeed("find /home/u/demo/data -type f").split())
        assert not before <= compacted, "an unrestricted compact freed nothing, so the check above proves nothing"

    with subtest("the key opens no other repository"):
        out = client.fail("borg-job-demo init --encryption=none ssh://u@box/./elsewhere 2>&1")
        assert "path not allowed" in out, out
  '';
}
