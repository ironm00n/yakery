# The store a VM check's node boots with, from the flake's test packages: an image whose files are
# root's, writable, with its paths registered, as on a host.
{ pkgs }:
pkgs.testers.runNixOSTest {
  name = "vm-store";

  nodes.machine.virtualisation.graphics = false;

  testScript = ''
    machine.wait_for_unit("multi-user.target")
    system = machine.succeed("readlink -f /run/current-system").strip()

    with subtest("it is the image, and its files are root's"):
        fstypes = set(machine.succeed("findmnt -rno FSTYPE /nix/.ro-store").split())
        assert fstypes == {"erofs"}, fstypes
        owner = machine.succeed(f"stat -c %u {system}").strip()
        assert owner == "0", owner

    with subtest("its paths are registered"):
        machine.succeed("systemctl is-active register-nix-paths.service")
        machine.succeed(f"nix-store --query --hash {system}")

    with subtest("it takes new paths"):
        machine.succeed("echo kept > /tmp/kept && nix-store --add /tmp/kept")
  '';
}
