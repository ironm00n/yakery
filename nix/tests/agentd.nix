# agentd under the VM's own socket units: systemd binds and names its sockets, agentd starts again for a
# connection queued while it was stopped, and refuses to run without them.
{ pkgs }:
let
  inherit (pkgs) lib;
  agentd = pkgs.callPackage ../../crates/rust.nix { crate = "agentd"; };
  agent-host = pkgs.callPackage ../../crates/rust.nix { crate = "agent-host"; };
  agents = import ../vms/agents/agents.nix { inherit lib; };
  key = "/etc/agentd.key";
  apiAddress = "127.0.0.9";
  refusals = "journalctl -u agentd -p warning -o cat | grep -cx \\\"api tunnel: uid $(id -u stranger) is not an agent\\\"";
in
pkgs.testers.runNixOSTest {
  name = "agentd";

  nodes.vm = {
    imports = [
      (import ../vms/agents/agentd.nix {
        inherit key apiAddress;
        quotaDevice = "/dev/vda";
        sessions = 1;
        claude = lib.getExe' pkgs.coreutils "false";
      })
    ];
    # the control socket's AF_VSOCK, with CID 1 for this machine itself
    boot.kernelModules = [ "vsock_loopback" ];
    environment.etc."agentd.key" = {
      text = "0123456789abcdef0123456789abcdef";
      mode = "0400";
    };
    users.users = {
      alpha = {
        isNormalUser = true;
        inherit (agents.alpha) uid;
      };
      stranger.isNormalUser = true;
    };
    environment.systemPackages = [
      agent-host
      pkgs.socat
    ];
  };

  testScript = ''
    def connect(user, to):
        vm.succeed(f"su {user} -c 'socat -u /dev/null {to}'")

    def refused():
        return int(vm.succeed("${refusals} || true").strip())

    vm.wait_for_unit("sockets.target")
    vm.wait_for_unit("agentd.service")

    with subtest("systemd holds the sockets, the api one open to every user"):
        vm.succeed("test \"$(stat -c '%a %U' /run/agentd/api.sock)\" = '666 root'")
        vm.succeed("test $(ss -Hlx | grep -c /run/agentd/api.sock) = 1")
        vm.succeed("ss -Hlxp | grep /run/agentd/api.sock | grep -q '\"systemd\",pid=1,'")
        vm.succeed("ss -Hltnp | grep '${apiAddress}:443' | grep -q '\"systemd\",pid=1,'")
        vm.succeed("systemctl is-active agentd-control.socket")

    with subtest("a user that is no agent is refused on both api sockets, at warning priority"):
        connect("stranger", "UNIX-CONNECT:/run/agentd/api.sock")
        vm.wait_until_succeeds("test $(${refusals}) -ge 1")
        connect("stranger", "TCP:${apiAddress}:443")
        vm.wait_until_succeeds("test $(${refusals}) -ge 2")

    with subtest("stopped, agentd starts again for a connection that queued"):
        vm.succeed("systemctl stop agentd.service")
        before = refused()
        connect("stranger", "UNIX-CONNECT:/run/agentd/api.sock")
        vm.wait_for_unit("agentd.service")
        vm.wait_until_succeeds(f"test $(${refusals}) -gt {before}")
        vm.succeed("test -S /run/agentd/api.sock")

    with subtest("the control socket takes a vsock connection, and drops a peer that is not the host"):
        status, out = vm.execute("agentctl --cid 1 --key ${key} wake alpha hello 2>&1")
        assert status == 1 and ("early eof" in out or "Broken pipe" in out), out

    with subtest("without systemd's sockets agentd refuses to run"):
        status, out = vm.execute(
            "${lib.getExe' agentd "agentd"} --agents /dev/null --key ${key} --claude /bin/false"
            " --mkdir /bin/false --quota-device /dev/null --sessions 1 2>&1"
        )
        assert status == 1, out
        assert "agentd: systemd passed no socket named api; agentd runs from its socket units" in out, out
  '';
}
