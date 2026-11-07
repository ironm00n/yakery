{ pkgs }:
let
  keys = import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs;
  # Holds the MiB it is given, every page touched: bytearray(n) alone gets lazy zero pages.
  hog = pkgs.writeText "hog.py" ''
    import sys
    held = b"\x01" * (int(sys.argv[1]) << 20)
  '';
  common = {
    imports = [ ../../bundles/claude-agent.nix ];
    bundles.claude-agent = {
      enable = true;
      owner = "ironmoon";
      memoryMax = "300M";
      sessionMemoryMax = "100M";
    };
    users.users.ironmoon.isNormalUser = true;
    environment.systemPackages = [ pkgs.python3 ];
    virtualisation.memorySize = 1024;
    # the test instrumentation panics on any OOM, and the caps' kills are what this test checks
    boot.kernel.sysctl."vm.panic_on_oom" = 0;
  };
in
pkgs.testers.runNixOSTest {
  name = "claude-agent";

  nodes.desktop = common;
  nodes.laptop = {
    imports = [ common ];
    services.openssh.enable = true;
    bundles.claude-agent.authorizedKeys = [ keys.snakeOilPublicKey ];
  };

  testScript = ''
    asAgent = lambda cmd: f"su claude-agent -c 'XDG_RUNTIME_DIR=/run/user/1500 {cmd}'"
    sliceOoms = lambda: int(desktop.succeed("sed -n 's/^oom //p' /sys/fs/cgroup/user.slice/user-1500.slice/memory.events.local"))

    def refused(cmd, reason):
        code, out = desktop.execute(f"{cmd} 2>&1")
        assert code != 0 and reason in out, out

    start_all()
    for m in (desktop, laptop):
        m.wait_for_unit("multi-user.target")
        m.wait_for_unit("user@1500.service")

    with subtest("its own key, made at boot, is its own and private"):
        desktop.wait_for_unit("claude-agent-ssh-key.service")
        desktop.succeed("[ \"$(stat -c '%U %a' /home/claude-agent/.ssh/id_ed25519)\" = 'claude-agent 600' ]")

    with subtest("nothing in his home reaches it"):
        desktop.succeed("install -d -m 700 -o ironmoon /home/ironmoon/.ssh")
        desktop.succeed("install -m 600 -o ironmoon /dev/null /home/ironmoon/.ssh/id_ed25519")
        desktop.succeed("install -m 644 -o ironmoon /dev/null /home/ironmoon/notes.md")
        refused(asAgent("cat /home/ironmoon/.ssh/id_ed25519"), "Permission denied")
        refused(asAgent("cat /home/ironmoon/notes.md"), "Permission denied")

    with subtest("he becomes the agent; the agent becomes no one"):
        out = desktop.succeed("su ironmoon -c 'sudo -n -u claude-agent id -un'").strip()
        assert out == "claude-agent", out
        rules = desktop.execute("sudo -l -U claude-agent")[1]
        assert "is not allowed to run sudo" in rules, rules

    with subtest("a session's scope is killed alone at its cap"):
        env = desktop.succeed(asAgent("systemctl --user show-environment"))
        assert "CLAUDE_SESSION_MEMORY_MAX=100M" in env, env
        desktop.succeed(asAgent("systemd-run --user --scope --unit=neighbour sleep 300 >/dev/null 2>&1 &"))
        desktop.wait_until_succeeds(asAgent("systemctl --user is-active neighbour.scope"))
        desktop.fail(asAgent("systemd-run --user --scope --unit=capped -p MemoryMax=100M python3 ${hog} 200"))
        result = desktop.succeed(asAgent("systemctl --user show capped.scope -p Result --value")).strip()
        assert result == "oom-kill", result
        assert sliceOoms() == 0
        desktop.succeed(asAgent("systemctl --user is-active neighbour.scope"))

    with subtest("the account together is killed at its cap, and below it nothing is"):
        desktop.succeed(asAgent("systemd-run --user --scope python3 ${hog} 50"))
        desktop.fail(asAgent("systemd-run --user --scope python3 ${hog} 400"))
        assert sliceOoms() > 0

    with subtest("its key logs in to itself on the laptop and never to him"):
        laptop.wait_for_unit("sshd.service")
        desktop.succeed("install -m 600 -o claude-agent ${keys.snakeOilPrivateKey} /home/claude-agent/.ssh/test_key")
        # -F /dev/null: ssh refuses the system config's include from a store that isn't root's, as under xb
        ssh = "ssh -F /dev/null -i /home/claude-agent/.ssh/test_key -o StrictHostKeyChecking=no -o BatchMode=yes"
        out = desktop.succeed(asAgent(f"{ssh} claude-agent@laptop id -un")).strip()
        assert out == "claude-agent", out
        refused(asAgent(f"{ssh} ironmoon@laptop true"), "Permission denied (publickey")
  '';
}
