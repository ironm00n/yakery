{ pkgs }:
let
  inherit (import (pkgs.path + "/nixos/tests/ssh-keys.nix") pkgs)
    snakeOilEd25519PrivateKey
    snakeOilEd25519PublicKey
    ;

  borg = "${pkgs.borgbackup}/bin/borg";

  box = import ./borg-box.nix { inherit pkgs; };

  # Same shape as the Hetzner forced command: the box, not the hop, is the
  # boundary that survives a compromised hop.
  forcedCommand = "${borg} serve --append-only --restrict-to-repository /home/u/repo";

  # borg appends `<host> borg serve ...` to BORG_RSH; the shim ignores that.
  viaHop = "BORG_RSH=${pkgs.writeShellScript "borg-hop-rsh" "exec ${pkgs.socat}/bin/socat STDIO TCP:hop:8023"}";
in
pkgs.testers.runNixOSTest {
  name = "borg-hop-bundle";

  defaults = {
    virtualisation.graphics = false;
    _module.args.my-lib = import ../../lib { inherit (pkgs) lib; };
    programs.ssh.knownHosts.box.publicKey = box.hostKey.public;
  };

  nodes = {
    box = box.node;

    # vlan 2 is the interface the relay must not answer on.
    hop = {
      imports = [ ../../bundles/borg-hop.nix ];
      virtualisation.vlans = [
        1
        2
      ];
      bundles.borg-hop = {
        enable = true;
        interfaces = [ "eth1" ];
        target = {
          host = "box";
          user = "u";
          hostPublicKey = box.hostKey.public;
          remotePath = borg;
          repositories = [ "/home/u/repo" ];
        };
      };
    };

    client = {
      virtualisation.vlans = [
        1
        2
      ];
      environment.systemPackages = [
        pkgs.borgbackup
        pkgs.socat
      ];
    };
  };

  testScript = ''
    start_all()
    box.wait_for_unit("sshd.service")
    hop.wait_for_unit("borg-hop.socket")
    hop.wait_until_succeeds("test -s /var/lib/borg-hop/id_ed25519.pub")

    hop_key = hop.succeed("cat /var/lib/borg-hop/id_ed25519.pub").strip()
    box.succeed(
        "install -d -m 700 -o u -g users /home/u/.ssh",
        f"echo 'command=\"${forcedCommand}\",restrict {hop_key}' > /home/u/.ssh/authorized_keys",
        "echo '${snakeOilEd25519PublicKey}' >> /home/u/.ssh/authorized_keys",
        "chown u:users /home/u/.ssh/authorized_keys",
        "chmod 600 /home/u/.ssh/authorized_keys",
    )
    client.succeed(
        "install -d -m 700 /root/.ssh",
        "install -m 600 ${snakeOilEd25519PrivateKey} /root/.ssh/id_ed25519",
    )

    with subtest("the relay carries the borg RPC end to end"):
        client.succeed("${viaHop} borg init --encryption=none ssh://hop/./repo")
        client.succeed("${viaHop} borg create ssh://hop/./repo::a1 /etc/hostname")
        listing = client.succeed(
            "BORG_RELOCATED_REPO_ACCESS_IS_OK=yes borg list --short --remote-path ${borg} ssh://u@box/./repo"
        )
        assert listing.split() == ["a1"], listing

    with subtest("the box's restriction holds through the relay"):
        out = client.fail("${viaHop} borg init --encryption=none ssh://hop/./elsewhere 2>&1")
        assert "path not allowed" in out, out

    with subtest("the relay is not reachable off the listed interfaces"):
        hop_vlan2 = hop.succeed("ip -4 -o addr show dev eth2 | awk '{print $4}' | cut -d/ -f1").strip()
        client.succeed(f"ping -c 1 {hop_vlan2}")
        client.fail(f"socat -T 3 STDIO TCP:{hop_vlan2}:8023,connect-timeout=3 < /dev/null")
  '';
}
