{
  pkgs,
  inputs,
  my-lib,
}:
let
  inherit (pkgs) lib;
  inherit (import (pkgs.path + "/nixos/tests/ssh-keys.nix") pkgs)
    snakeOilEd25519PrivateKey
    snakeOilEd25519PublicKey
    ;

  rootPassphrase = "root-test-passphrase";
  agentsKey = "agents-test-key-0123456789abcdef";
  borgPassphrase = "borg-test-passphrase";
  passwordHash = "$6$desktoptestsalt$JoJQG/59ZEi.aZrvjt.wYm2LBIiDz2UGO8YkWn4igy1Mrzc8eKoMMNLNjSiXxgKgqOeEoAFu/CSZP3sHU1nTW1";

  testSecrets = ./desktop-boot/secrets.yaml;

  desktopIp = "192.168.1.10";
  pastDefaultDeviceTimeout = 130;

  installed = inputs.nixpkgs.lib.nixosSystem {
    specialArgs = {
      inherit my-lib;
      inputs = inputs // {
        secrets.lib = {
          luks.desktop-agents = testSecrets;
          borg.desktop = testSecrets;
        };
      };
    };
    modules = [
      inputs.disko.nixosModules.disko
      inputs.sops-nix.nixosModules.sops
      ../../hosts/desktop/disks.nix
      ../../hosts/desktop/boot.nix
      ../../hosts/desktop/restore.nix
      (
        { modulesPath, ... }:
        {
          imports = [
            (modulesPath + "/testing/test-instrumentation.nix")
            (modulesPath + "/profiles/qemu-guest.nix")
          ];
          nixpkgs.pkgs = pkgs;
          networking.hostName = "desktop";
          system.stateVersion = "26.05";
          documentation.enable = false;

          disko.devices.disk.nvme.device = "/dev/vdb";
          disko.devices.disk.agents.device = "/dev/vdc";
          disko.devices.disk.nvme.content.partitions.esp.size = lib.mkForce "512M";
          disko.devices.lvm_vg.desktop.lvs.swap.size = "1G";

          sops.age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];
          # the host-store virtiofs share refuses to freeze, which aborts hibernation
          boot.blacklistedKernelModules = [ "virtiofs" ];
          boot.initrd.systemd.settings.Manager.DefaultDeviceTimeoutSec = lib.mkForce "infinity";
          # a resumed guest never re-announces the test backdoor
          systemd.services.systemd-hibernate.serviceConfig.ExecStartPost = [
            (pkgs.writeShellScript "hibernation-witness" ''
              echo "hibernation-witness: $(cat /run/hibernation-marker)" > /dev/kmsg
            '')
          ];
          users.users.ironmoon.isNormalUser = true;

          networking.usePredictableInterfaceNames = false;
          boot.initrd.systemd.network.networks."10-ether" = lib.mkForce {
            matchConfig.Name = "eth1";
            address = [ "${desktopIp}/24" ];
          };
          boot.initrd.network.ssh.authorizedKeys = [ snakeOilEd25519PublicKey ];

          systemd.services.desktop-restore.environment.BORG_RSH = lib.mkForce (
            toString (
              pkgs.writeShellScript "local-borg-serve" ''
                cd /var/lib/restore-test && exec ${lib.getExe pkgs.borgbackup} serve
              ''
            )
          );
        }
      )
    ];
  };

  installedSystem = installed.config.system.build.toplevel;
  luksPart = "/dev/disk/by-partlabel/disk-nvme-luks";
  sbctlBin = lib.getExe pkgs.sbctl;
in
pkgs.testers.runNixOSTest {
  name = "desktop-boot";

  defaults.virtualisation.graphics = false;

  nodes = {
    desktop =
      { pkgs, ... }:
      {
        virtualisation = {
          useBootLoader = true;
          mountHostNixStore = true;
          additionalPaths = [ installedSystem ];
          useEFIBoot = true;
          efi.OVMF = pkgs.OVMFFull;
          efi.keepVariables = true;
          tpm.enable = true;
          memorySize = 3072;
          emptyDiskImages = [
            {
              size = 8192;
              driveConfig.deviceExtraOpts.bootindex = "0";
            }
            1024
          ];
          vlans = [ 1 ];
        };
        boot.loader.systemd-boot.enable = true;
        nix.settings.substituters = lib.mkForce [ ];
        environment.systemPackages = with pkgs; [
          cryptsetup
          jq
          nixos-install-tools
          sbctl
        ];
      };

    laptop = {
      virtualisation.vlans = [ 1 ];
      environment.etc."ssh-key" = {
        source = snakeOilEd25519PrivateKey;
        mode = "0600";
      };
    };
  };

  testScript = ''
    import json

    def efivar(name):
        return int(desktop.succeed(f"od -An -tu1 -j4 -N1 /sys/firmware/efi/efivars/{name}-8be4df61-93ca-11d2-aa0d-00e098032b8c").strip())

    def tpm_pcrs():
        meta = json.loads(desktop.succeed("cryptsetup luksDump --dump-json-metadata ${luksPart}"))
        return sorted(tuple(t["tpm2-pcrs"]) for t in meta["tokens"].values() if t["type"] == "systemd-tpm2")

    def assert_installed_boot():
        desktop.wait_for_unit("multi-user.target")
        assert "/dev/mapper/desktop-root" in desktop.succeed("findmnt -no SOURCE /")
        assert efivar("SecureBoot") == 1, "booted without Secure Boot enforcement"

    start_all()
    desktop.wait_for_unit("multi-user.target")

    with subtest("old system boots in Setup Mode"):
        assert efivar("SetupMode") == 1
        assert efivar("SecureBoot") == 0

    with subtest("our keys enrolled from the old system: option ROMs pinned by hash, no Microsoft"):
        desktop.succeed("${sbctlBin} create-keys")
        db = desktop.succeed("cd $(mktemp -d) && ${sbctlBin} enroll-keys --tpm-eventlog --export esl >/dev/null && od -An -tx1 -v db.esl | tr -d ' \\n'")
        assert "2616c4c14c509240aca941f936934328" in db, "no option-ROM hash in the db sbctl would enroll"
        desktop.succeed("${sbctlBin} enroll-keys --tpm-eventlog")
        assert efivar("SetupMode") == 0
        desktop.fail("grep -a -q Microsoft /sys/firmware/efi/efivars/db-* /sys/firmware/efi/efivars/KEK-*")

    with subtest("disko lays out both disks"):
        desktop.succeed("printf %s ${rootPassphrase} > /tmp/root.key")
        desktop.succeed("printf %s ${agentsKey} > /tmp/agents.key")
        desktop.succeed("${installed.config.system.build.diskoScript} >&2")
        desktop.succeed("findmnt /mnt/boot")
        desktop.succeed("findmnt /mnt/var/lib/agents")
        assert desktop.succeed("blkid -o value -s TYPE /dev/desktop/swap").strip() == "swap"

    with subtest("install with extra files, as nixos-anywhere does"):
        desktop.succeed("install -D -m 600 ${snakeOilEd25519PrivateKey} /mnt/etc/ssh/ssh_host_ed25519_key")
        desktop.succeed("ssh-keygen -q -t ed25519 -N ''' -f /mnt/etc/ssh/initrd_ssh_host_ed25519_key")
        desktop.succeed("mkdir -p /mnt/var/lib && cp -a /var/lib/sbctl /mnt/var/lib/")
        desktop.succeed("nixos-install --system ${installedSystem} --root /mnt --no-root-passwd --no-channel-copy >&2")

    with subtest("limine signed itself with the carried-over keys"):
        status = json.loads(desktop.succeed("nixos-enter --root /mnt -c '${sbctlBin} status --json'"))
        assert not status["setup_mode"], status
        assert efivar("SetupMode") == 0
        verify = desktop.succeed("nixos-enter --root /mnt -c '${sbctlBin} verify' 2>&1")
        assert "efi/limine/bootx64.efi is signed" in verify.lower(), verify
        assert "efi/boot/bootx64.efi is signed" in verify.lower(), verify

    with subtest("TPM slot without a PCR policy"):
        desktop.succeed("systemd-cryptenroll --unlock-key-file=/tmp/root.key --tpm2-device=auto --tpm2-pcrs= ${luksPart}")
        assert tpm_pcrs() == [()], tpm_pcrs()

    desktop.succeed("umount -R /mnt && swapoff -a && sync")
    desktop.shutdown()

    with subtest("first boot: Secure Boot enforced, root unlocked by the TPM, agents disk by sops"):
        desktop.start()
        assert_installed_boot()
        desktop.wait_for_unit("systemd-cryptsetup@agents.service")
        assert "/dev/mapper/agents" in desktop.succeed("findmnt -no SOURCE /var/lib/agents")
        active = desktop.succeed("swapon --show=NAME --noheadings | xargs -r readlink -f").split()
        assert desktop.succeed("readlink -f /dev/desktop/swap").strip() in active, active
        assert "resume=/dev/desktop/swap" in desktop.succeed("cat /proc/cmdline")

    with subtest("bind the TPM to PCR 0+7"):
        desktop.succeed("printf %s ${rootPassphrase} > /run/root.key")
        desktop.succeed("systemd-cryptenroll --unlock-key-file=/run/root.key --tpm2-device=auto --tpm2-pcrs=0+7 --wipe-slot=tpm2 ${luksPart}")
        assert tpm_pcrs() == [(0, 7)], tpm_pcrs()
        desktop.succeed("rm /run/root.key")
        desktop.shutdown()
        desktop.start()
        assert_installed_boot()

    with subtest("hibernate and resume through the TPM-unlocked swap volume"):
        desktop.succeed("echo survived > /run/hibernation-marker")
        desktop.execute("systemctl hibernate >&2 &", check_return=False)
        desktop.wait_for_shutdown()
        desktop.start()
        desktop.wait_for_console_text("hibernation-witness: survived")
        desktop.crash()
        desktop.start()
        assert_installed_boot()
        desktop.fail("test -e /run/hibernation-marker")

    with subtest("restore: user state from pre-luks-*, whole container stores from containers-*"):
        borg = "cd /tmp/src && BORG_PASSPHRASE=${borgPassphrase} ${lib.getExe pkgs.borgbackup}"
        desktop.succeed(
            "mkdir -p /var/lib/restore-test /tmp/src/home/ironmoon/.config /tmp/src/home/ironmoon/.local/share/containers /tmp/src/etc/nixos",
            "echo hi > /tmp/src/home/ironmoon/.config/marker",
            "echo stale > /tmp/src/home/ironmoon/.local/share/containers/layer",
            "echo flake > /tmp/src/etc/nixos/flake.nix",
            "printf 'root:!:1::::::\\nironmoon:${passwordHash}:1::::::\\n' > /tmp/src/etc/shadow",
            "cd /var/lib/restore-test && BORG_PASSPHRASE=${borgPassphrase} ${lib.getExe pkgs.borgbackup} init --encryption=repokey desktop",
            f"{borg} create /var/lib/restore-test/desktop::pre-luks-2026-01-01 .",
            f"cd /tmp/src && echo newer > home/ironmoon/.config/marker && {borg} create /var/lib/restore-test/desktop::pre-luks-2026-01-02 .",
            "mkdir -p /tmp/src/var/lib/docker && echo vol > /tmp/src/var/lib/docker/volume && chown 999:999 /tmp/src/var/lib/docker/volume",
            "echo fresh > /tmp/src/home/ironmoon/.local/share/containers/layer",
            f"{borg} create /var/lib/restore-test/desktop::containers-2026-01-02 var/lib/docker home/ironmoon/.local/share/containers",
        )
        desktop.succeed("systemctl start desktop-restore.service")
        assert desktop.succeed("cat /home/ironmoon/.config/marker").strip() == "newer"
        assert desktop.succeed("cat /var/lib/docker/volume").strip() == "vol"
        assert desktop.succeed("stat -c %u:%g /var/lib/docker/volume").strip() == "999:999"
        assert desktop.succeed("cat /home/ironmoon/.local/share/containers/layer").strip() == "fresh"
        assert desktop.succeed("cat /etc/nixos/flake.nix").strip() == "flake"
        assert desktop.succeed("getent shadow ironmoon").split(":")[1] == "${passwordHash}"

    with subtest("without the TPM slot, initrd SSH takes the passphrase"):
        desktop.succeed("printf %s ${rootPassphrase} > /run/root.key")
        desktop.succeed("systemd-cryptenroll --unlock-key-file=/run/root.key --wipe-slot=tpm2 ${luksPart}")
        assert tpm_pcrs() == [], tpm_pcrs()
        desktop.shutdown()
        desktop.start()
        ssh = "ssh -i /etc/ssh-key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2222 root@${desktopIp}"
        laptop.wait_until_succeeds(f"{ssh} 'ls /run/systemd/ask-password/ask.*'", timeout=300)
        laptop.sleep(${toString pastDefaultDeviceTimeout})
        laptop.succeed(f"{{ printf %s ${rootPassphrase}; echo; }} | {ssh} -tt systemd-tty-ask-password-agent")
        assert_installed_boot()
  '';
}
