{ pkgs }:
let
  probe =
    python:
    pkgs.writeShellScript "probe" ''
      exec ${python}/bin/python3 -I ${pkgs.writeText "probe.py" ''
        import ctypes, resource, threading

        libc = ctypes.CDLL(None)
        attr = ctypes.create_string_buffer(64)
        libc.pthread_getattr_default_np(attr)
        size = ctypes.c_size_t()
        libc.pthread_attr_getstacksize(attr, ctypes.byref(size))
        thread = threading.Thread(target=lambda: None)
        thread.start()
        thread.join()
        limit = resource.getrlimit(resource.RLIMIT_STACK)[0]
        print("unlimited" if limit == resource.RLIM_INFINITY else limit, size.value)
      ''} 2>&1
    '';
  probe64 = probe pkgs.python3;
  probe32 = probe pkgs.pkgsi686Linux.python3;
in
pkgs.testers.runNixOSTest {
  name = "stack-bundle";

  nodes.machine = {
    imports = [ ../../bundles/stack ];
    virtualisation.graphics = false;
    bundles.stack.enable = true;
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
      linger = true;
      extraGroups = [ "wheel" ];
    };
    security.sudo.wheelNeedsPassword = false;
  };

  testScript = ''
    machine.wait_for_unit("user@1000.service")

    def probe(cmd, expected):
        out = machine.succeed(cmd).strip()
        assert out == expected, f"{cmd}: {out!r}"

    with subtest("login shells get an unlimited stack and 8 MiB threads, 64- and 32-bit"):
        probe("su - alice -c ${probe64}", "unlimited 8388608")
        probe("su - alice -c ${probe32}", "unlimited 8388608")

    with subtest("so do the user manager's units"):
        probe("systemd-run --machine=alice@ --user --wait --pipe ${probe64}", "unlimited 8388608")

    with subtest("system services, root logins and sudo from an unlimited session keep 8 MiB"):
        probe("systemd-run --wait --pipe ${probe64}", "8388608 8388608")
        probe("su - root -c ${probe64}", "8388608 8388608")
        probe("su - alice -c 'sudo ${probe64}'", "8388608 8388608")
  '';
}
