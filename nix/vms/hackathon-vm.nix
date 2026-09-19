# A throwaway NixOS guest for running sponsor-provided or otherwise untrusted coding harnesses
# (Cursor and friends) during a hackathon, without handing them the host home. The appboxes bundle is
# the wrong tool here: distrobox mounts the real home alongside the box home and exposes the host
# root at /run/host, so a harness that reads absolute paths still reaches ~/.ssh, ~/.claude and
# /etc/nixos. A guest reaches exactly one host directory, the share.
#
# Run (no `nixos-rebuild switch`, so this works on the laptop untouched):
#   nix run /etc/nixos#hackathon-vm
# HACKATHON_SHARE (default ~/hackathon-share, created if missing) is the one host directory the guest
# sees, at ~/share; NIX_DISK_IMAGE (default $XDG_STATE_HOME/hackathon-vm/disk.qcow2) is its disk.
# Login: hacker / hacker. Super+Space is the launcher; agents are on PATH (claude, codex, pi),
# and `hackathon-apps` fetches the ones nobody packages (hackathon-vm-apps.nix).
#
# The clipboard moves one selection per keypress in either direction (hackathon-vm-clip.nix):
# Super+Shift+V sends the host's to the guest, Super+Ctrl+V fetches the guest's back. Neither
# mirrors, so nothing crosses unless you press the key.
#
# Verified 2026-09-18 by booting it repeatedly (`-display egl-headless`, which `virtio-vga-gl`
# requires — plain `-display none` refuses to start):
#   · the share mounts and the host can read what the guest writes into it
#   · `cursor --version` reports 3.19.13 (3.17.21 before the lock bump) run as the unprivileged user
#   · `NIXOS_OZONE_WL=1` reaches the login environment, so Electron is native Wayland here
#   · sway is installed and getty autologin is set to `hacker`
# NOT verified: that the tty1 login actually lands you in sway. Five probe iterations failed to
# answer it and each failure was the instrument, not the guest — a systemd unit has its own PATH
# and no PAM environment, `pgrep | head` always exits 0 so its `|| echo NO` never fired, and
# `runuser` creates no logind session so `sway --validate` aborts on a missing XDG_RUNTIME_DIR
# before it reads a config. Open the window once and look; if it drops to a shell, type `sway`.
#
# An earlier X11 + i3 + lightdm version of this file also worked (same three checks green). It was
# dropped because the guest session may as well match the host's and the cursor wrapper is
# Wayland-aware. From that round: `services.xserver.enable` already selects lightdm, so the
# explicit display-manager line it had was redundant.
#
# The 2026-09-18 lock bump (nixpkgs 0968519 → efe6f07) moved Linux-host shares from 9p to virtiofs
# and added `sharedDirectories.*.writable`, default false: the share went read-only, and an unset
# $HACKATHON_SHARE now kills virtiofsd before qemu starts instead of failing inside qemu.
{
  inputs,
  system,
}:
let
  pkgs = import inputs.nixpkgs {
    inherit system;
    config.allowUnfree = true; # cursor is unfree; scoped to this guest, not to any host
  };
  clip = import ./hackathon-vm-clip.nix { inherit pkgs; };
  apps = import ./hackathon-vm-apps.nix { inherit pkgs; };
  # the same build the host installs through home-manager, and the one .deb nobody has to unpack
  codex-desktop = inputs.codex-desktop-linux.packages.${system}.codex-desktop;

  # Electron's safeStorage refuses to persist a login when no org.freedesktop.secrets provider
  # answers, which is every Electron app here; the CLI writes ~/.claude/.credentials.json and
  # doesn't care. An empty password gives an unencrypted keyring that unlocks without a prompter
  # — there is no gcr prompter in this session, and a passworded keyring nobody can unlock is
  # the same as no keyring. The disk this lands on is the throwaway one.
  keyring = pkgs.writeShellApplication {
    name = "hackathon-vm-keyring";
    runtimeInputs = with pkgs; [
      gnome-keyring
      dbus
      coreutils
    ];
    text = ''
      dir="''${XDG_DATA_HOME:-$HOME/.local/share}/keyrings"
      mkdir -p "$dir"
      if [ ! -e "$dir/login.keyring" ]; then
        printf '[keyring]\ndisplay-name=login\nlock-on-idle=false\nlock-after=false\n' \
          > "$dir/login.keyring"
        printf 'login' > "$dir/default"
      fi
      eval "$(gnome-keyring-daemon --unlock --components=secrets,pkcs11 < /dev/null)"
      dbus-update-activation-environment --systemd GNOME_KEYRING_CONTROL SSH_AUTH_SOCK
    '';
  };
in
inputs.nixpkgs.lib.nixosSystem {
  inherit system;
  modules = [
    # `virtualisation.*` and `system.build.vm` live here; a plain nixosSystem has neither.
    "${inputs.nixpkgs}/nixos/modules/virtualisation/qemu-vm.nix"
    # qemu-vm builds the root drive itself and exposes no per-drive hook, so patch its
    # declaration instead of mkForce-ing the whole list back into existence by hand — the list
    # keeps tracking the module. Without discard a qcow2 only grows: one day of agent builds
    # wrote 202 GB through a 128 GiB filesystem holding 108, and the image had allocated 126.
    (
      { lib, ... }:
      {
        options.virtualisation.qemu.drives = lib.mkOption {
          apply = map (
            drive:
            if drive.name != "root" then
              drive
            else
              drive
              // {
                driveExtraOpts = drive.driveExtraOpts // {
                  discard = "unmap";
                  detect-zeroes = "unmap";
                };
              }
          );
        };
      }
    )
    ../../bundles/ok-color.nix
    (
      { config, lib, ... }:
      {
        nixpkgs.pkgs = pkgs;

        # Same boot as the hosts (hosts/common/interactive): blue OKs and kitty's VT palette.
        bundles.ok-color.enable = true;
        console.colors = import ../kitty-palette.nix { inherit pkgs; };

        # Five harnesses, each with subagents, is the workload — so the guest gets the machine and
        # the host keeps what is left: 14 of 16 threads, 48 of 60 GiB. The guest's RAM is a memfd,
        # so that is a ceiling rather than a reservation and the host can swap it. Neither can
        # grow without a restart — there is no balloon device and no -smp maxcpus. The disk only
        # caps how far the image grows, but the drive has no discard=unmap, so qcow2 allocation
        # is a high-water mark that never falls: a guest that has freed a build tree still holds
        # those host blocks. Budget it as fully allocated. An image made before a bump keeps the
        # old ceiling; `block_resize` on the HMP grows it live, `qemu-img resize` while down.
        virtualisation = {
          memorySize = 49152;
          cores = 14;
          diskSize = 262144;
          graphics = true;
          # virtio-gpu with GL, or Electron software-renders and the editor feels like 2009.
          # No grab-on-hover: a GTK keyboard grab is a shortcuts inhibitor on Wayland, and
          # Hyprland honours it by dropping every bind, including the passthrough toggle. The
          # submap is the one way to hand Super chords to the guest; the pointer is absolute.
          qemu.options = [
            "-device virtio-vga-gl"
            # killactive only sends a close request, and window-close=off makes gtk ignore it,
            # so a stray Super+Q over the guest does nothing rather than ending the machine the
            # agents live in. Deliberate exits still work: shut down inside the guest, or `quit`
            # on the monitor tab (Ctrl+Alt+2). Hyprland's forcekillactive still kills qemu.
            "-display gtk,gl=on,window-close=off"
            "-device ${clip.device}"
          ];
          # vhost-user-fs needs the guest RAM in a memfd; efe6f07 defaults this to false, master
          # has since made it follow virtiofs. Drop once the pin passes that.
          qemu.enableSharedMemory = true;
          # The store is a read-only erofs image on virtio-blk, not a virtiofs share. virtiofsd
          # holds an O_PATH fd per inode it has ever served and frees it only on a FUSE FORGET,
          # which this guest never sends because 48 GiB means it never evicts anything — so the
          # store, which is every library load, ran a userspace daemon out of descriptors in an
          # hour and the guest could not dlopen libc. File handles would cost no descriptors but
          # need CAP_DAC_READ_SEARCH, and handing the file server DAC bypass to fix a resource
          # leak is a worse trade than deleting the server. Costs a ~10 GiB image build per
          # launch; in exchange the guest sees only its own closure instead of the host's store.
          useNixStoreImage = true;
          # writableStore follows mountHostNixStore, which useNixStoreImage turns off — without
          # this the guest gets a bind mount and a read-only store, and `nix profile add` is
          # back to failing. Overlay over the erofs image instead, upper on the root disk.
          writableStore = true;
          # The store overlay's upper layer defaults to tmpfs, so everything nix writes lives in
          # RAM and is gone at reboot — while the flake eval cache in the persistent home still
          # names those .drv paths, and the next boot reads a derivation that no longer exists.
          # On the disk instead: `nix profile add` survives a reboot and isn't capped by memory.
          writableStoreUseTmpfs = false;
          sharedDirectories.share = {
            source = "$HACKATHON_SHARE";
            target = "/home/hacker/share";
            writable = true;
          };
          # Growing the image with `qemu-img resize` leaves the filesystem at its old size, and
          # the initrd is the one place that can extend it before anything has mounted it rw.
          fileSystems."/".autoResize = true;
        };

        # discard only lets the guest hand blocks back; something has to ask. Weekly is the
        # stock interval and useless at this write rate — the whole disk turns over in a day.
        services.fstrim = {
          enable = true;
          interval = "hourly";
        };

        # Four agents at once is the normal load here, so memory pressure is the expected state
        # and has to end in a kill rather than a stall. With no swap the kernel cannot page out
        # anonymous memory at all, so it evicts executable pages instead and reads them straight
        # back over virtiofs — the machine livelocks with nothing ever OOM-killed. zram gives
        # reclaim somewhere to go, and earlyoom shoots the biggest process before the stall gets
        # that far. Not bundles.oom: it watches user@.service, and sway starts from the tty1
        # login shell, so everything interesting lives in session-1.scope beside it, not under it.
        # A quarter rather than the default half: zram lives in the same 48 GiB it is swapping
        # out of, and ~3x compression already makes 12 GiB worth roughly 36 GiB of cold pages.
        # virtiofsd holds an O_PATH descriptor per inode the guest has looked up and closes it
        # only on a FUSE FORGET, which the guest sends when it evicts that inode. With 48 GiB
        # there is never enough pressure to evict, so the table ratchets until it hits the fd
        # budget virtiofsd fixed at startup from its rlimit — after which every dlopen in the
        # guest fails with ENFILE and even bash will not start, while the guest's own fd table
        # sits at ~4k. Raising the host rlimit afterwards does nothing; the budget is already
        # computed. File handles would need no descriptors at all, but open_by_handle_at wants
        # CAP_DAC_READ_SEARCH and virtiofsd runs unprivileged here, so --inode-file-handles
        # silently falls back to descriptors. What is left is to make the guest forget on a
        # schedule rather than under pressure that never comes. Only unused inodes are dropped,
        # so this cannot pull a library out from under a running process.
        systemd.services.forget-store-inodes = {
          description = "Drop the dentry and inode caches so virtiofsd can close descriptors";
          serviceConfig = {
            Type = "oneshot";
            ExecStart = "${lib.getExe' pkgs.procps "sysctl"} --write vm.drop_caches=2";
          };
        };
        systemd.timers.forget-store-inodes = {
          wantedBy = [ "timers.target" ];
          timerConfig = {
            OnBootSec = "10min";
            OnUnitActiveSec = "10min";
          };
        };

        zramSwap.enable = true;
        zramSwap.memoryPercent = 25;
        services.earlyoom = {
          enable = true;
          freeMemThreshold = 8;
          freeSwapThreshold = 10;
          # Killing the compositor takes the other three agents' windows with it.
          extraArgs = [
            "--avoid"
            "^(sway|Xwayland|systemd)$"
          ];
        };

        # The runner forks virtiofsd before anything checks $HACKATHON_SHARE, so an unset variable
        # surfaces as a dead vhost-user socket; resolve it here. NIX_DISK_IMAGE would otherwise
        # land in whatever directory `nix run` was typed from.
        system.build.sandbox = pkgs.writeShellApplication {
          name = "hackathon-vm";
          text = ''
            export HACKATHON_SHARE="''${HACKATHON_SHARE:-$HOME/hackathon-share}"
            state="''${XDG_STATE_HOME:-$HOME/.local/state}/hackathon-vm"
            export NIX_DISK_IMAGE="''${NIX_DISK_IMAGE:-$state/disk.qcow2}"
            mkdir -p "$HACKATHON_SHARE" "$(dirname "$NIX_DISK_IMAGE")"
            exec ${lib.getExe config.system.build.vm} "$@"
          '';
        };

        users.users.hacker = {
          isNormalUser = true;
          initialPassword = "hacker";
          extraGroups = [ "wheel" ];
        };
        security.sudo.wheelNeedsPassword = false;

        # Wayland, not X: the nixpkgs cursor wrapper honours NIXOS_OZONE_WL and passes
        # --ozone-platform-hint=auto, so Electron is native here and there is no reason for the
        # guest's session to differ from the host's. No display manager — getty autologin on tty1
        # and sway from the shell profile is fewer moving parts than greetd for a throwaway guest.
        programs.sway = {
          enable = true;
          wrapperFeatures.gtk = true;
        };
        environment.sessionVariables.NIXOS_OZONE_WL = "1";
        # virglrenderer ≤ 1.3.0 ignores Y_0_TOP on the guest's cursor plane, so a hardware cursor
        # renders upside down (qemu #2315); software cursors sidestep the plane entirely.
        environment.sessionVariables.WLR_NO_HARDWARE_CURSORS = "1";
        # Super+Space matches the host's launcher bind and, like every other chord except
        # Super+Escape, reaches the guest from inside the passthrough submap. The stock config
        # binds it to `focus mode_toggle` and sway raises its config-error bar for a silent
        # overwrite, so drop that one first. The clipboard listeners need this session's
        # WAYLAND_DISPLAY, so they start here too.
        environment.etc."sway/config.d/hackathon.conf".text = ''
          unbindsym Mod4+space
          bindsym Mod4+space exec ${lib.getExe pkgs.fuzzel}
          exec ${lib.getExe keyring}
          exec ${lib.getExe clip.serve}
        '';
        services.getty.autologinUser = "hacker";
        programs.bash.loginShellInit = ''
          if [ "$(tty)" = /dev/tty1 ] && [ -z "$WAYLAND_DISPLAY" ]; then
            exec sway
          fi
        '';

        # fish is not POSIX, so it is nobody's login shell here — hacker keeps
        # users.defaultUserShell and the tty1 exec above stays bash's. The module rather than
        # the package: direnv's fish hook is a programs.fish.interactiveShellInit definition,
        # which nothing writes to /etc/fish unless the fish module is enabled.
        programs.fish.enable = true;
        # Hooks bash and fish; the module's zsh and xonsh halves find no shell to write into.
        programs.direnv.enable = true;

        environment.systemPackages = with pkgs; [
          apps
          claude-code
          codex
          codex-desktop
          pi-coding-agent
          code-cursor
          firefox # sponsor tools want an OAuth round-trip
          foot
          warp-terminal
          fuzzel
          git
          jujutsu
          jjui
          nodejs_22
          cargo
          rustc
          rust-analyzer
          gcc
          pkg-config
          ripgrep
          fd
          jq
          helix
          wl-clipboard
        ];

        # There is no systemwide FHS in NixOS — buildFHSEnv is per-command — so this is the
        # closest thing: nix-ld answers for the loader a foreign binary asks for, envfs answers
        # for /usr/bin and /bin shebangs. Enough to run vendor installers and unpacked .debs
        # (hackathon-apps) without packaging them. Guest only; the host keeps neither.
        programs.nix-ld.enable = true;
        programs.nix-ld.libraries = with pkgs; [
          alsa-lib
          at-spi2-atk
          at-spi2-core
          atk
          cairo
          cups
          dbus
          expat
          fontconfig
          freetype
          gdk-pixbuf
          glib
          gtk3
          libdrm
          libgbm
          libGL
          libnotify
          libpulseaudio
          libsecret
          libuuid
          libxkbcommon
          mesa
          nspr
          nss
          pango
          libx11
          libxscrnsaver
          libxcomposite
          libxcursor
          libxdamage
          libxext
          libxfixes
          libxi
          libxrandr
          libxrender
          libxtst
          libxcb
        ];
        services.envfs.enable = true;
        # hackathon-apps drops launchers here, and so do the vendor installers (homeBinInPath is
        # ~/bin, which is not where any of them write).
        environment.localBinInPath = true;

        nix.settings.experimental-features = [
          "nix-command"
          "flakes"
        ];
        nix.settings.trusted-users = [ "hacker" ];

        # Serial console, so a headless boot test sees the journal rather than six lines of getty
        # banner. The GTK window stays the real console for interactive use.
        boot.kernelParams = [ "console=ttyS0,115200" "console=tty0" ];

        # Self-report instead of logging in over the serial line to check by hand: writes into the
        # share, which also proves it works in the direction that matters (the host can read it).
        systemd.services.hackathon-selftest = {
          description = "report whether this guest is actually usable";
          wantedBy = [ "multi-user.target" ];
          after = [ "home-hacker-share.mount" "getty@tty1.service" ];
          serviceConfig.Type = "oneshot";
          path = with pkgs; [
            util-linux
            code-cursor
            coreutils
            systemd
            bash # cursor's launcher shells out; without this the version probe reports no `sh`
          ];
          script = ''
            report() { echo "selftest: $*" > /dev/console; echo "$*" >> /home/hacker/share/selftest.txt; }
            : > /home/hacker/share/selftest.txt || echo "selftest: SHARE NOT WRITABLE" > /dev/console
            report "share mounted: $(mountpoint -q /home/hacker/share && echo yes || echo NO)"
            report "cursor: $(runuser -u hacker -- cursor --version 2>&1 | head -1)"
            # Probe the login session, not this unit: a systemd unit has its own PATH and gets no
            # PAM session variables, so `command -v` and $NIXOS_OZONE_WL here say nothing about
            # what the autologin shell sees. Two earlier probe iterations reported exactly that
            # artifact as a guest defect.
            report "sway installed: $(test -x /run/current-system/sw/bin/sway && echo yes || echo NO)"
            report "ozone in login env: $(runuser -l hacker -c 'echo ''${NIXOS_OZONE_WL:-unset}' 2>&1 | tail -1)"
            # `sway --validate` answers "would it start at all" without a display, which a pgrep
            # can't distinguish from "hasn't got there yet". Note pgrep|head also always exits 0,
            # so the previous `|| echo NO` could never fire — instrument bugs, not guest bugs, in
            # three of the four probe iterations so far.
            report "sway config valid: $(runuser -l hacker -c 'sway --validate' 2>&1 | tail -1)"
            report "sway process: $(pgrep -c sway || true) running"
            report "autologin user: $(systemctl show getty@tty1 -p ExecStart --value | grep -o 'hacker' | head -1)"
            report "hacker uid: $(id -u hacker)"
            report "done"
          '';
        };

        # qemu-vm.nix:1474 turns timesyncd off outright, so the guest reads the emulated RTC once
        # at boot and then free-runs — which under saturated vCPUs drifts hours, and agents that
        # timestamp a shared board make ordering decisions out of those numbers. Plain definition
        # over there, so mkForce is enough.
        services.timesyncd.enable = lib.mkForce true;

        networking.hostName = "hackathon-vm";
        networking.firewall.enable = true;
        documentation.enable = false;
        # man-db keys off documentation.man.enable, not documentation.enable, so the guest
        # carried a man reading pages it never installed — and once fish is on, its mkDefault
        # index came too: a man-paths buildEnv of every systemPackages man output in a closure
        # rebuilt as an erofs image per launch, plus a boot-time mandb unit and user.
        documentation.man.enable = false;
        system.stateVersion = lib.trivial.release;
      }
    )
  ];
}
