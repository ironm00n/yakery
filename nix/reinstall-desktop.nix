{
  lib,
  writeShellApplication,
  coreutils,
  gawk,
  gnugrep,
  gnutar,
  iproute2,
  iputils,
  netcat-openbsd,
  nixos-anywhere,
  nix,
  openssh,
  sbctl,
  sbsigntool,
  sops,
  desktop,
  secrets,
}:
let
  inherit (desktop.config.system.build) toplevel diskoScript;
  inherit (desktop.config.disko.devices) disk;
  esp = desktop.config.boot.loader.efi.efiSysMountPoint;
  efivar = name: "/sys/firmware/efi/efivars/${name}-8be4df61-93ca-11d2-aa0d-00e098032b8c";
  dbVar = "/sys/firmware/efi/efivars/db-d719b2cb-3d3a-4596-a3bc-dad00e67656f";
  luksPart = "/dev/disk/by-partlabel/${disk.nvme.content.partitions.luks.label}";
  rootSecret = "${secrets}/secrets/luks/desktop-root.yaml";
  agentsSecret = secrets.lib.luks.desktop-agents;
  sandisk = "/dev/disk/by-id/usb-SanDisk_Extreme_SSD_31393530445A343031313834-0:0";
  desktopMac = "24:4b:fe:59:3f:6b";
  hop = "hetzner-cx23-1.h.im.exposed";
  efiCertSha256Guid = "2616c4c14c509240aca941f936934328";
  microsoftHex = "4d6963726f736f6674";

  # the kexec installer and the new system take DHCP leases of their own, so every
  # connection finds the desktop by MAC over IPv6 link-local on $REINSTALL_LINK
  viaMac = writeShellApplication {
    name = "via-mac";
    runtimeInputs = [
      gawk
      iproute2
      iputils
      netcat-openbsd
    ];
    text = ''
      port=''${1:-22}
      link=''${REINSTALL_LINK:?}
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        ping -6 -c1 -W1 "ff02::1%$link" >/dev/null 2>&1 || true
        for addr in $(ip -6 neigh show dev "$link" | awk '$3 == "${desktopMac}" && $1 ~ /^fe80:/ && $NF !~ /FAILED|INCOMPLETE/ {print $1}'); do
          if ping -6 -c1 -W1 "$addr%$link" >/dev/null 2>&1; then
            exec nc "$addr%$link" "$port"
          fi
        done
        sleep 1
      done
      echo "via-mac: ${desktopMac} is not on $link" >&2
      exit 1
    '';
  };
in
writeShellApplication {
  name = "reinstall-desktop";
  excludeShellChecks = [
    "SC2016"
    "SC2029"
  ];
  runtimeInputs = [
    coreutils
    gnugrep
    gnutar
    iproute2
    nixos-anywhere
    nix
    openssh
    sbsigntool
    sops
  ];
  text = ''
    target=''${1:?usage: reinstall-desktop <desktop-lan-ip>}
    work=$(mktemp -d -p "''${XDG_RUNTIME_DIR:-/tmp}" reinstall-desktop.XXXXXX)
    stage=preflight
    tpm_unlocked=1

    finish() {
      local rc=$?
      if [ "$stage" = finished ]; then rm -rf "$work"; return; fi
      printf '\n\033[31mstopped during: %s (exit %s)\033[0m\n' "$stage" "$rc" >&2
      printf 'kept %s (netbird identity, SSH host keys, LUKS secrets, logs); delete it when done\n' "$work" >&2
      case $stage in
        kexec)
          printf 'nothing is wiped and no Secure Boot keys are enrolled: if the installer never answers, power-cycle the desktop and the old system boots as before\n' >&2 ;;
        install)
          printf 'the disks may already be wiped: do not reboot the installer; ssh -i %s/install_key -o ProxyCommand="${lib.getExe viaMac} %%p" root@%s\n' "$work" "$target" >&2 ;;
      esac
      case $stage in
        enroll)
          printf 'the new system is installed; if it stops booting, clear the Secure Boot keys in the BIOS and it boots unenforced\n' >&2 ;;&
        enroll|unbound)
          printf 'a TPM slot without a PCR policy may still be enrolled; once booted, rebind it:\n  systemd-cryptenroll --unlock-key-file=<root.key> --tpm2-device=auto --tpm2-pcrs=0+7 --wipe-slot=tpm2 ${luksPart}\n' >&2 ;;
      esac
    }
    trap finish EXIT

    REINSTALL_LINK=$(ip -o route get "$target" | grep -o 'dev [^ ]*' | cut -d' ' -f2)
    export REINSTALL_LINK
    export NIX_SSHOPTS="-o ProxyCommand=${lib.getExe viaMac}"

    step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
    warn() { printf '\033[33m%s\033[0m\n' "$*"; }
    die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
    ssh_opts=(-o BatchMode=yes -o ServerAliveInterval=15 -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new -o "ProxyCommand=${lib.getExe viaMac} %p")
    on() { ssh "''${ssh_opts[@]}" "root@$target" "$@"; }
    on_installer() { ssh "''${ssh_opts[@]}" -i "$work/install_key" -o IdentitiesOnly=yes "root@$target" "$@"; }
    on_initrd() { ssh "''${ssh_opts[@]}" -p 2222 -o "UserKnownHostsFile=$work/known_hosts.initrd" "root@$target" "$@"; }
    setup_mode=${efivar "SetupMode"}
    secure_boot=${efivar "SecureBoot"}
    efivar() { "$1" "od -An -tu1 -j4 -N1 $2" | tr -d ' '; }
    efi_hex() { on "od -An -tx1 -v $1 | tr -d ' \n'"; }
    guid_bytes() { local g=''${1//-/}; printf %s "''${g:6:2}''${g:4:2}''${g:2:2}''${g:0:2}''${g:10:2}''${g:8:2}''${g:14:2}''${g:12:2}''${g:16:16}"; }
    boot_id() { "$@" 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null || true; }

    reboot_and_wait() {
      local before
      before=$(boot_id "$1")
      [ -n "$before" ] || die "could not read the boot id before rebooting; nothing was rebooted"
      "$1" "$2"
      wait_for_boot "$before"
    }

    # initrd sshd also answers during a successful TPM unlock; only a pending ask-password is a failure
    wait_for_boot() {
      local before=$1 deadline=$((SECONDS + 1800))
      until [ "$(boot_id on)" != "$before" ] && [ "$(on 'findmnt -no SOURCE /' 2>/dev/null)" = /dev/mapper/desktop-root ]; do
        [ $SECONDS -lt $deadline ] || die "the desktop did not come back within 30 minutes; check the monitor"
        if on_initrd 'ls /run/systemd/ask-password/ask.* >/dev/null 2>&1' 2>/dev/null; then
          warn "the initrd is asking for the passphrase (TPM did not unlock); answering over initrd SSH"
          { cat "$work/root.key"; echo; } | on_initrd -tt systemd-tty-ask-password-agent >/dev/null 2>&1 || true
          tpm_unlocked=0
        fi
        sleep 5
      done
    }

    backup() {
      local since
      since=$(on 'date +%s')
      on "/home/ironmoon/.local/state/borg-seed/seed $1"
      on "while systemctl is-active -q borg-seed-$1; do sleep 15; done; journalctl -u borg-seed-$1 --since @$since -o cat" \
        | tee "$work/backup-$1.log"
      grep -Eq 'terminating with (success|warning) status, rc [01]$' "$work/backup-$1.log" \
        || die "the $1 backup did not finish cleanly; nothing has been touched yet"
    }

    step "preflight on $target"
    [ "$(on hostname)" = desktop ] || die "$target is not the desktop"
    [ "$(efivar on "$setup_mode")" = 1 ] || die "firmware is not in Setup Mode: clear the Secure Boot keys in the BIOS first"
    on 'test -c /dev/tpmrm0' || die "no TPM on the desktop"
    ${lib.concatMapStrings (d: ''
      on 'test -b ${d.device}' || die "${d.device} is missing"
    '') (lib.attrValues disk)}
    on 'test ! -e ${sandisk}' || die "unplug the SanDisk first"
    on 'test -x /home/ironmoon/.local/state/borg-seed/seed' || die "the borg seed launcher is missing"
    on 'test ! -e /var/lib/sbctl' || die "/var/lib/sbctl already exists on the desktop; move it aside first"

    step "decrypting the LUKS secrets (YubiKey)"
    printf %s "$(sops -d --extract '["passphrase"]' ${rootSecret})" > "$work/root.key"
    printf %s "$(sops -d --extract '["key"]' ${agentsSecret})" > "$work/agents.key"
    [ -s "$work/root.key" ] && [ -s "$work/agents.key" ] || die "a decrypted LUKS secret is empty"

    stage=backup
    step "final backups: everything, then the container stores without excludes"
    backup desktop
    backup desktop-containers

    step "collecting the netbird identity and SSH host keys, generating the initrd host key and an install key"
    mkdir -p "$work/extra/etc/ssh"
    on 'tar -C / -cf - var/lib/netbird etc/ssh/ssh_host_ed25519_key etc/ssh/ssh_host_ed25519_key.pub etc/ssh/ssh_host_rsa_key etc/ssh/ssh_host_rsa_key.pub' \
      | tar -C "$work/extra" -xf -
    [ -s "$work/extra/etc/ssh/ssh_host_ed25519_key" ] || die "could not collect the desktop's SSH host key; nothing has been touched yet"
    ssh-keygen -q -t ed25519 -N "" -C initrd@desktop -f "$work/extra/etc/ssh/initrd_ssh_host_ed25519_key"
    printf '[%s]:2222 %s\n' "$target" "$(cut -d' ' -f1,2 "$work/extra/etc/ssh/initrd_ssh_host_ed25519_key.pub")" > "$work/known_hosts.initrd"
    ssh-keygen -q -t ed25519 -N "" -C reinstall-desktop -f "$work/install_key"

    anywhere() {
      nixos-anywhere --store-paths ${diskoScript} ${toplevel} \
        --target-host "root@$target" -i "$work/install_key" --ssh-option "ProxyCommand=${lib.getExe viaMac}" "$@"
    }

    step "unloading the GPU drivers, so the kexec'd kernel does not inherit a live NVIDIA GPU"
    on 'mkdir -p /run/modprobe.d
    for m in nvidia nvidia_drm nvidia_modeset nvidia_uvm evdi; do echo "install $m /run/current-system/sw/bin/false"; done > /run/modprobe.d/kexec-no-gpu.conf
    systemctl stop display-manager.service displaylink-server.service 2>/dev/null
    for c in /sys/class/vtconsole/vtcon*/bind; do echo 0 > "$c" 2>/dev/null; done
    for i in 1 2 3; do
      modprobe -r nvidia_drm nvidia_modeset nvidia_uvm evdi 2>/dev/null; rmmod nvidia_uvm 2>/dev/null; modprobe -r nvidia 2>/dev/null
      lsmod | grep -qE "^(nvidia|evdi) " || exit 0
      sleep 2
    done
    exit 1' || die "the GPU drivers would not unload; nothing has been touched yet"

    step "Secure Boot keys: created on the desktop and checked against the TPM event log; enrolled after the first boot"
    nix copy --to "ssh-ng://root@$target" ${sbctl}
    on '${lib.getExe sbctl} create-keys'
    db=$(on 'cd "$(mktemp -d)" && ${lib.getExe sbctl} enroll-keys --tpm-eventlog --export esl >/dev/null && od -An -tx1 -v db.esl | tr -d " \n"')
    grep -q ${efiCertSha256Guid} <<<"$db" \
      || die "the TPM event log yields no option-ROM hash; enrolling would lock out the GPU's display driver"
    on 'tar -C / -cf - var/lib/sbctl' | tar -C "$work/extra" -xf -
    owner=$(guid_bytes "$(cat "$work/extra/var/lib/sbctl/GUID")")

    stage=kexec
    step "kexec into the installer (the monitor stays black until the new system boots)"
    anywhere --phases kexec

    step "checking the installer can reach the TPM and write EFI variables (nothing wiped yet, no keys enrolled)"
    on_installer 'test -c /dev/tpmrm0' \
      || die "no TPM in the installer; nothing is wiped"
    on_installer 'findmnt -no OPTIONS /sys/firmware/efi/efivars | grep -qw rw' \
      || die "efivarfs is not writable in the installer; nothing is wiped"

    step "ready to wipe: installer checks passed, nothing wiped yet, Secure Boot keys get enrolled after the first boot"
    read -r -p "type wipe to erase ${
      lib.concatMapStringsSep " and " (d: d.device) (lib.attrValues disk)
    } and install: " answer </dev/tty
    [ "$answer" = wipe ] || die "not wiping; the desktop is in the kexec installer with its disks untouched"

    stage=install
    step "installing: disko, nixos-install"
    anywhere --phases disko,install \
      --copy-host-keys --extra-files "$work/extra" \
      --disk-encryption-keys /tmp/root.key "$work/root.key" \
      --disk-encryption-keys /tmp/agents.key "$work/agents.key"

    step "checking both boot paths are signed with our db key before rebooting"
    for efi in EFI/limine/BOOTX64.EFI EFI/BOOT/BOOTX64.EFI; do
      on_installer "cat /mnt${esp}/$efi" > "$work/boot.efi" || die "$efi is missing from the ESP"
      sbverify --cert "$work/extra/var/lib/sbctl/keys/db/db.pem" "$work/boot.efi" >/dev/null \
        || die "$efi is not signed with our db key"
    done

    step "temporary TPM slot without a PCR policy, so the first boots unlock unattended"
    stage=unbound
    on_installer "systemd-cryptenroll --unlock-key-file=/tmp/root.key --tpm2-device=auto --tpm2-pcrs= ${luksPart}" \
      || warn "TPM enrollment in the installer failed; the first boot will be unlocked over initrd SSH"

    step "rebooting into the new system (Secure Boot not enforced yet)"
    reboot_and_wait on_installer 'umount -R /mnt; swapoff -a; nohup sh -c "sleep 3; reboot" >/dev/null 2>&1 &'

    stage=enroll
    step "enrolling our Secure Boot keys from the new system: GPU option ROM pinned by hash, no Microsoft certificates"
    [ "$(efivar on "$setup_mode")" = 1 ] || die "the firmware is no longer in Setup Mode"
    nix copy --to "ssh-ng://root@$target" ${sbctl}
    db=$(on 'cd "$(mktemp -d)" && ${lib.getExe sbctl} enroll-keys --tpm-eventlog --export esl >/dev/null && od -An -tx1 -v db.esl | tr -d " \n"')
    grep -q ${efiCertSha256Guid} <<<"$db" \
      || die "the TPM event log yields no option-ROM hash; enrolling would lock out the GPU's display driver"
    on '${lib.getExe sbctl} enroll-keys --tpm-eventlog'
    # this firmware reports SetupMode=1 until its next reset, so check what was written instead
    for var in ${efivar "PK"} ${efivar "KEK"} ${dbVar}; do
      hex=$(efi_hex "$var")
      grep -q "$owner" <<<"$hex" || die "''${var##*/} does not hold our key after enrolling"
      ! grep -q ${microsoftHex} <<<"$hex" || die "''${var##*/} holds a Microsoft certificate"
    done
    grep -q ${efiCertSha256Guid} <<<"$hex" || die "db does not hold the GPU option-ROM hash"

    step "rebooting so the firmware enforces Secure Boot"
    reboot_and_wait on 'nohup sh -c "sleep 3; reboot" >/dev/null 2>&1 &'
    [ "$(efivar on "$secure_boot")" = 1 ] \
      || die "the firmware does not enforce Secure Boot after enrolling"

    step "binding the TPM to PCR 0+7 before anything long-running"
    on 'umask 077; cat > /run/root.key' < "$work/root.key"
    on "systemd-cryptenroll --unlock-key-file=/run/root.key --tpm2-device=auto --tpm2-pcrs=0+7 --wipe-slot=tpm2 ${luksPart}; rc=\$?; rm -f /run/root.key; exit \$rc"

    stage=restore
    step "restoring user state through the hop"
    on 'timeout 600 sh -c "until resolvectl query ${hop} >/dev/null 2>&1; do sleep 5; done"' \
      || die "netbird never resolved the hop; restore with: ssh root@$target systemctl start desktop-restore"
    restore_rc=0
    on 'systemctl start desktop-restore.service' || restore_rc=$?
    on 'journalctl -u desktop-restore -b -o cat | tail -n 5; n=$(wc -l < /var/lib/desktop-restore/warnings.log); echo "$n warning lines; first 100:"; head -n 100 /var/lib/desktop-restore/warnings.log' \
      | tee "$work/restore.log"
    [ "$restore_rc" = 0 ] || die "restore failed (see above); TPM is already bound, the system is usable"

    stage=final
    step "final reboot: the root must unlock from the TPM alone"
    tpm_unlocked=1
    reboot_and_wait on 'nohup sh -c "sleep 3; reboot" >/dev/null 2>&1 &'
    [ "$tpm_unlocked" = 1 ] || die "the PCR 0+7 binding did not unlock the root; it was unlocked by passphrase instead"
    on 'findmnt /var/lib/agents >/dev/null' || die "the agent disk is not mounted"

    stage=finished
    step "done"
    echo "add this to ~/.ssh/known_hosts for future initrd unlocks:"
    cat "$work/known_hosts.initrd"
  '';
}
