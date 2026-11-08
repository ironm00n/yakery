{ pkgs }:
let
  keys = import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs;
  # Holds the MiB it is given, every page touched: bytearray(n) alone gets lazy zero pages.
  hog = pkgs.writeText "hog.py" ''
    import sys
    held = b"\x01" * (int(sys.argv[1]) << 20)
  '';
  # The interceptor, for what the bundle wires around it: it writes its CA and the policy, dispatch rule and pool token
  # it was started with, opens its tap socket and takes connections on its proxy port.
  tap = pkgs.runCommand "anthropic-tap-stand-in" { } ''
    install -D -m 755 ${pkgs.writeScript "anthropic-tap" ''
      #!${pkgs.python3}/bin/python3
      import os, socket
      env, state = os.environ, os.environ["STATE_DIR"]
      open(f"{state}/ca.pem", "w").write("cert\n")
      key = os.open(f"{state}/ca.key", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
      os.write(key, b"key\n")
      creds = env.get("CREDENTIALS_DIRECTORY")
      pool = open(f"{creds}/pool").read().strip() if creds else ""
      open(f"{state}/seen", "w").write(f"{env.get('FLAGS')} {env.get('DISPATCH')} {pool}\n")
      tap = socket.socket(socket.AF_UNIX)
      tap.bind(env["TAP_SOCK"])
      os.chmod(env["TAP_SOCK"], int(env["TAP_SOCK_MODE"], 8))
      tap.listen()
      host, port = env["LISTEN_PROXY"].rsplit(":", 1)
      proxy = socket.create_server((host, int(port)))
      while True:
          proxy.accept()[0].close()
    ''} $out/bin/anthropic-tap
    install -D -m 644 ${pkgs.writeText "policy.env" "FLAGS=a=true,b=false\n"} $out/share/anthropic-tap/policy.env
  '';
  listen = pkgs.writeText "listen.py" ''
    import socket, sys
    server = socket.create_server(("127.0.0.1", int(sys.argv[1])))
    while True:
        server.accept()[0].close()
  '';
  connect = pkgs.writeText "connect.py" ''
    import socket, sys
    family, address = (socket.AF_UNIX, sys.argv[1]) if "/" in sys.argv[1] else (socket.AF_INET, ("127.0.0.1", int(sys.argv[1])))
    socket.socket(family).connect(address)
  '';
  common =
    { lib, ... }:
    {
      imports = [
        ../../users/claude-agent/user.nix
        ../../users/claude-agent/tap.nix
      ];
      systemd.slices."user-1500".sliceConfig.MemoryMax = lib.mkForce "300M";
      systemd.services."user@1500".environment.CLAUDE_SESSION_MEMORY_MAX = lib.mkForce "100M";
      # what notes' `just agent-tap-install` and the deploy's token step leave behind
      systemd.tmpfiles.rules = [ "L+ /nix/var/nix/profiles/claude-agent-tap - - - - ${tap}" ];
      environment.etc."claude-agent/pool-token".text = "token\n";
      users.users.ironmoon.uid = 1000;
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
    users.users.claude-agent.openssh.authorizedKeys.keys = [ keys.snakeOilPublicKey ];
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

    with subtest("its tap runs under a uid of its own, with the policy file and the pool token the account can't read"):
        desktop.wait_for_unit("claude-agent-tap.service")
        seen = desktop.succeed("cat /var/lib/claude-agent-tap/seen").strip()
        assert seen == "a=true,b=false pool token", seen
        unit = desktop.succeed("cat /etc/anthropic-tap/claude-agent")
        desktop.succeed(f"systemctl is-active {unit}")
        user = desktop.succeed("ps -o user:32= -p $(systemctl show -p MainPID --value claude-agent-tap)").strip()
        assert user == "claude-agent-tap", user
        refused(asAgent("cat /run/credentials/claude-agent-tap.service/pool"), "Permission denied")

    with subtest("the account reads the tap's certificate and never its key"):
        desktop.succeed(asAgent("cat /var/lib/claude-agent-tap/ca.pem"))
        refused(asAgent("cat /var/lib/claude-agent-tap/ca.key"), "Permission denied")

    with subtest("only the account reaches its tap's port, and only he reads the tap"):
        desktop.wait_until_succeeds(asAgent("python3 ${connect} 8891"))
        refused("su ironmoon -c 'python3 ${connect} 8891'", "Connection refused")
        desktop.succeed("su ironmoon -c 'python3 ${connect} /run/claude-agent-tap/events.sock'")
        refused(asAgent("python3 ${connect} /run/claude-agent-tap/events.sock"), "Permission denied")

    with subtest("his own loopback services answer him and refuse the account"):
        desktop.succeed("su ironmoon -c 'python3 ${listen} 8384 >/dev/null 2>&1 &'")
        desktop.wait_until_succeeds("su ironmoon -c 'python3 ${connect} 8384'")
        refused(asAgent("python3 ${connect} 8384"), "Connection refused")

    with subtest("its key logs in to itself on the laptop and never to him"):
        laptop.wait_for_unit("sshd.service")
        desktop.succeed("install -m 600 -o claude-agent ${keys.snakeOilPrivateKey} /home/claude-agent/.ssh/test_key")
        ssh = "ssh -i /home/claude-agent/.ssh/test_key -o StrictHostKeyChecking=no -o BatchMode=yes"
        out = desktop.succeed(asAgent(f"{ssh} claude-agent@laptop id -un")).strip()
        assert out == "claude-agent", out
        refused(asAgent(f"{ssh} ironmoon@laptop true"), "Permission denied (publickey")
  '';
}
