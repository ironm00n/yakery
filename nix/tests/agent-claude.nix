{ pkgs }:
let
  shared = import ./agent-node.nix { inherit pkgs; };
  inherit (shared)
    account
    uid
    connect
    listen
    hog
    ;
  tapUser = "${account.name}-claude-tap";
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
in
pkgs.testers.runNixOSTest {
  name = "agent-claude";

  nodes.desktop =
    { lib, ... }:
    {
      imports = [
        shared.node
        ../../users/agent/claude/tap.nix
        ../../users/agent/claude/sessions.nix
      ];
      systemd.services."user@${uid}".environment.CLAUDE_SESSION_MEMORY_MAX = lib.mkForce "100M";
      # what notes' `just agent-tap-install` and the deploy's token step leave behind
      systemd.tmpfiles.rules = [ "L+ /nix/var/nix/profiles/${tapUser} - - - - ${tap}" ];
      environment.etc."${tapUser}/pool-token".text = "token\n";
    };

  testScript = shared.python + ''
    booted(desktop)

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

    with subtest("its tap runs under a uid of its own, with the policy file and the pool token the account can't read"):
        desktop.wait_for_unit("${tapUser}.service")
        seen = desktop.succeed("cat /var/lib/${tapUser}/seen").strip()
        assert seen == "a=true,b=false pool token", seen
        unit = desktop.succeed("cat /etc/anthropic-tap/${account.name}")
        desktop.succeed(f"systemctl is-active {unit}")
        user = desktop.succeed("ps -o user:32= -p $(systemctl show -p MainPID --value ${tapUser})").strip()
        assert user == "${tapUser}", user
        refused(asAgent("cat /run/credentials/${tapUser}.service/pool"), "Permission denied")

    with subtest("the account reads the tap's certificate and never its key"):
        desktop.succeed(asAgent("cat /var/lib/${tapUser}/ca.pem"))
        refused(asAgent("cat /var/lib/${tapUser}/ca.key"), "Permission denied")

    with subtest("only the account reaches its tap's port, and only he reads the tap"):
        desktop.wait_until_succeeds(asAgent("python3 ${connect} 8891"))
        refused("su ironmoon -c 'python3 ${connect} 8891'", "Connection refused")
        desktop.succeed("su ironmoon -c 'python3 ${connect} /run/${tapUser}/events.sock'")
        refused(asAgent("python3 ${connect} /run/${tapUser}/events.sock"), "Permission denied")

    with subtest("his own tap's ports answer him and refuse the account"):
        for port in (8888, 8889):
            desktop.succeed(f"su ironmoon -c 'python3 ${listen} {port} >/dev/null 2>&1 &'")
            desktop.wait_until_succeeds(f"su ironmoon -c 'python3 ${connect} {port}'")
            refused(asAgent(f"python3 ${connect} {port}"), "Connection refused")
  '';
}
