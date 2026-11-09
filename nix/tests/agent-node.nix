# What the agent account's checks share: a host with the account on it beside his, and the test script's helpers.
{ pkgs }:
rec {
  account = import ../../users/agent/account.nix;
  uid = toString account.uid;
  # Holds the MiB it is given, every page touched: bytearray(n) alone gets lazy zero pages.
  hog = pkgs.writeText "hog.py" ''
    import sys
    held = b"\x01" * (int(sys.argv[1]) << 20)
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

  node =
    { lib, ... }:
    {
      imports = [
        ../../users/agent/user.nix
        ../../users/agent/fence.nix
      ];
      systemd.slices."user-${uid}".sliceConfig.MemoryMax = lib.mkForce "300M";
      users.users.ironmoon = {
        uid = 1000;
        isNormalUser = true;
        linger = true;
      };
      environment.systemPackages = [ pkgs.python3 ];
      virtualisation.memorySize = 1024;
      # the test instrumentation panics on any OOM, and the caps' kills are what these checks check
      boot.kernel.sysctl."vm.panic_on_oom" = 0;
    };

  python = ''
    import shlex
    asAgent = lambda cmd: f"su ${account.name} -c 'XDG_RUNTIME_DIR=/run/user/${uid} {cmd}'"
    asHim = lambda cmd: "su ironmoon -c " + shlex.quote(cmd)
    sliceOoms = lambda: int(desktop.succeed("sed -n 's/^oom //p' /sys/fs/cgroup/user.slice/user-${uid}.slice/memory.events.local"))

    def refused(cmd, reason):
        code, out = desktop.execute(f"{cmd} 2>&1")
        assert code != 0 and reason in out, out

    def booted(*machines):
        start_all()
        for m in machines:
            m.wait_for_unit("multi-user.target")
            m.wait_for_unit("user@${uid}.service")
  '';
}
