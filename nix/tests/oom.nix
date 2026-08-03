{ pkgs }:
let
  # A package-provided user unit, so the bundle's lastResortUserUnits drop-in
  # has a real unit to attach to (the same shape as uwsm's wayland-wm@).
  shielded = pkgs.runCommand "shielded-unit" { } ''
    mkdir -p $out/lib/systemd/user
    cat > $out/lib/systemd/user/shielded.service <<EOF
    [Service]
    ExecStart=${pkgs.coreutils}/bin/sleep infinity
    EOF
  '';

  asAlice = cmd: "su alice -c 'XDG_RUNTIME_DIR=/run/user/1000 ${cmd}'";

  # Holds MemAvailable — the same gate oomd's swap kill checks — under
  # 160 MiB. A one-shot pin is not enough: the allocation burst leaves kswapd
  # swapping out bystander pages, which rebounds MemAvailable back over the
  # gate after the loop exits. MemorySwapMax=0 keeps the pinned pages from
  # swapping back out; the nonzero fill is what faults them in — bytearray(n)
  # gets lazy zero pages.
  ballast = pkgs.writeText "ballast.py" ''
    import time

    def available_kib():
        with open("/proc/meminfo") as meminfo:
            return next(
                int(line.split()[1])
                for line in meminfo
                if line.startswith("MemAvailable:")
            )

    keep = []
    while True:
        if available_kib() > 160 * 1024:
            keep.append(b"\x01" * (8 * 1024 * 1024))
        else:
            time.sleep(0.2)
  '';
in
pkgs.testers.runNixOSTest {
  name = "oom-bundle";

  nodes.machine = {
    imports = [ ../../bundles/oom.nix ];

    # 2 GiB so the swap subtest's "MemAvailable below 10%" band is wide
    # enough to sit in comfortably without courting the kernel OOM killer.
    virtualisation.memorySize = 2048;

    # The driver only passes -nographic when it sees no $DISPLAY, so without
    # this a driver run outside the sandbox pops a QEMU window per node.
    virtualisation.graphics = false;

    # A swapfile can't live on the default tmpfs root, so give the VM a real
    # ext4 root; autoFormat is x-systemd.makefs and needs the systemd stage 1,
    # which also means no filesystem label, hence the explicit rootDevice.
    virtualisation.useDefaultFilesystems = false;
    virtualisation.rootDevice = "/dev/vda";
    virtualisation.fileSystems."/" = {
      device = "/dev/vda";
      fsType = "ext4";
      autoFormat = true;
    };
    virtualisation.diskSize = 1024;
    boot.initrd.systemd.enable = true;

    swapDevices = [
      {
        device = "/var/swapfile";
        size = 128;
      }
    ];

    bundles.oom = {
      enable = true;
      pressureLimit = 20;
      pressureDuration = "2s";
      lastResortUserUnits = [ "shielded" ];
    };

    systemd.packages = [ shielded ];

    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
      linger = true;
    };

    environment.systemPackages = [ pkgs.attr ];
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("systemd-oomd.service")
    machine.wait_for_unit("user@1000.service")

    with subtest("swap is active"):
        machine.wait_for_unit("var-swapfile.swap")
        machine.succeed("swapon --show | grep /var/swapfile")

    with subtest("oomd monitors the user session and the root slice"):
        machine.wait_until_succeeds(
            "oomctl | grep -A5 'Memory Pressure Monitored CGroups' "
            "| grep '/user.slice/user-1000.slice/user@1000.service'",
            timeout=60,
        )
        machine.succeed("oomctl | grep -A2 'Swap Monitored CGroups' | grep 'Path: /$'")

    with subtest("sysrq is fully enabled"):
        assert machine.succeed("sysctl -n kernel.sysrq").strip() == "1"

    with subtest("ManagedOOMPreference reaches the cgroup as an xattr"):
        machine.succeed("${asAlice "systemctl --user start shielded.service"}")
        machine.succeed(
            "getfattr -n user.oomd_avoid "
            "/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/shielded.service"
        )

    with subtest("oomd kills a thrashing app and spares the shielded unit"):
        # MemorySwapMax=0 makes the hog stall instead of spilling to swap, so
        # the pressure monitor fires rather than racing the swap monitor.
        machine.succeed(
            "${asAlice "systemd-run --user --unit=hog --property=MemoryHigh=32M --property=MemorySwapMax=0 tail /dev/zero"}"
        )
        machine.wait_until_fails("${asAlice "systemctl --user is-active hog.service"}", timeout=90)
        machine.succeed("${asAlice "systemctl --user is-active shielded.service"}")

    with subtest("oomd kills the top swap consumer once swap runs out"):
        # The swap monitor only acts once MemAvailable AND free swap both drop
        # under 100% - SwapUsedLimit (default 10%), so pin most of RAM first.
        machine.succeed(
            "systemd-run --unit=ballast --property=MemorySwapMax=0 "
            "${pkgs.python3}/bin/python3 ${ballast}"
        )
        machine.wait_until_succeeds(
            "[ $(awk '/MemAvailable/ {print $2}' /proc/meminfo) -lt 180000 ]", timeout=120
        )
        # A system service sits outside user@1000, so only the root-slice swap
        # monitor can be what kills it.
        machine.succeed("systemd-run --unit=swaphog --property=MemoryHigh=32M tail /dev/zero")
        machine.wait_until_fails("systemctl is-active swaphog.service", timeout=120)
        result = machine.succeed("systemctl show -p Result --value swaphog.service").strip()
        assert result == "oom-kill", f"swaphog Result was {result!r}"
        machine.succeed(
            "journalctl -u systemd-oomd.service "
            "| grep 'Marked /system.slice/swaphog.service for killing due to memory used'"
        )
        # Victims are picked by swap usage: the far larger, swapless ballast
        # and the shielded unit must both survive.
        machine.succeed("systemctl is-active ballast.service")
        machine.succeed("${asAlice "systemctl --user is-active shielded.service"}")
  '';
}
