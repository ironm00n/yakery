{ pkgs }:
pkgs.testers.runNixOSTest {
  name = "blocked-kernel-modules";

  nodes.machine = {
    imports = [ ../../hosts/common/base/blocked-kernel-modules.nix ];
    users.users.alice.isNormalUser = true;
    environment.systemPackages = [ pkgs.socat ];
    virtualisation.graphics = false;
  };

  testScript = ''
    def connect(address):
        return machine.fail(f"su alice -c 'socat -u /dev/null {address}:9' 2>&1")

    machine.wait_for_unit("multi-user.target")
    machine.succeed("modinfo sctp sctp_diag")

    with subtest("an unprivileged SCTP socket loads no module"):
        for address in ["SCTP4-CONNECT:127.0.0.1", "SCTP6-CONNECT:[::1]"]:
            out = connect(address)
            assert "Protocol not supported" in out, out
        machine.fail("grep -qw ^sctp /proc/modules")

    with subtest("modprobe by name is refused"):
        machine.fail("modprobe sctp")
        machine.fail("modprobe sctp_diag")
        machine.fail("grep -qw ^sctp /proc/modules")

    with subtest("the same socket opens once sctp is loaded past the block"):
        machine.succeed("modprobe --ignore-install sctp")
        out = connect("SCTP4-CONNECT:127.0.0.1")
        assert "Connection refused" in out, out
  '';
}
