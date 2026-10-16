{
  config,
  lib,
  pkgs,
  utils,
  ...
}:
let
  agents = import ./agents.nix { inherit lib; };
  agentd = pkgs.callPackage ../../../packages/agentd/package.nix { };
  mac = "02:61:67:6e:74:01";
  mib = gib: gib * 1024;
  bytes = gib: mib gib * 1024 * 1024;
  volume = name: mountPoint: sizeGiB: mkfsArgs: {
    image = "/var/lib/agents/vm/${name}.img";
    label = name;
    inherit mountPoint;
    size = mib sizeGiB;
    direct = true;
    mkfsExtraArgs = [
      "-i"
      "8192"
    ]
    ++ mkfsArgs;
  };
  overlayDir = "/nix/.rw-store";
  overlay = volume "nix-rw" overlayDir 256 [ ];
  overlayDevice = "/dev/disk/by-label/${overlay.label}";
  buildDir = "${overlayDir}/build";
  home = volume "home" "/home" 640 [
    "-O"
    "quota"
  ];
in
{
  microvm = {
    hypervisor = "qemu";
    vcpu = 16;
    mem = mib 32;
    vsock = {
      cid = 4;
      ssh.enable = true;
    };
    storeDiskDirect = true;
    writableStoreOverlay = overlayDir;
    volumes = [
      overlay
      home
    ];
    interfaces = [
      {
        type = "macvtap";
        id = "vm-agents";
        inherit mac;
        macvtap = {
          link = "enp6s0";
          mode = "bridge";
        };
      }
    ];
  };

  # A kept overlay keeps GC's whiteouts over erofs paths that a later image ships again, and the nix
  # database on the tmpfs root forgets what the overlay holds; every boot starts both empty.
  boot.initrd.systemd = {
    extraBin."mkfs.ext4" = "${pkgs.e2fsprogs}/bin/mkfs.ext4";
    services.fresh-store-overlay = {
      description = "Empty store overlay";
      unitConfig.DefaultDependencies = false;
      requires = [ "${utils.escapeSystemdPath overlayDevice}.device" ];
      after = [ "${utils.escapeSystemdPath overlayDevice}.device" ];
      before = [
        "${utils.escapeSystemdPath "/sysroot${overlayDir}"}.mount"
        "systemd-fsck@${utils.escapeSystemdPath overlayDevice}.service"
      ];
      requiredBy = [ "${utils.escapeSystemdPath "/sysroot${overlayDir}"}.mount" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = "mkfs.ext4 -F -q -L ${overlay.label} ${lib.escapeShellArgs overlay.mkfsExtraArgs} ${overlayDevice}";
    };
  };
  boot.initrd.kernelModules = [ "quota_v2" ];

  fileSystems."/home" = {
    neededForBoot = true;
    options = [ "usrquota" ];
  };

  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    min-free = bytes 32;
    max-free = bytes 64;
    build-dir = buildDir;
  };
  systemd.tmpfiles.settings."10-nix-build".${buildDir}.d.mode = "0755";

  users.mutableUsers = false;
  users.groups.agents = { };
  users.users =
    lib.mapAttrs (_: agent: {
      isNormalUser = true;
      inherit (agent) uid;
      group = "agents";
    }) agents
    // {
      root.openssh.authorizedKeys.keys = import ../../../hosts/common/admin-ssh-keys.nix;
    };

  systemd.services.agent-quotas = {
    description = "Home quotas from the agent list";
    wantedBy = [ "multi-user.target" ];
    serviceConfig.Type = "oneshot";
    script = lib.concatMapAttrsStringSep "\n" (
      name: agent:
      "${lib.getExe' pkgs.quota "setquota"} -u ${name} 0 ${
        toString (mib agent.homeQuotaGiB * 1024)
      } 0 0 /home"
    ) agents;
  };

  systemd.services.agentd = {
    description = "Agent sessions and their API socket";
    wantedBy = [ "multi-user.target" ];
    path = [ config.systemd.package ];
    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe' agentd "agentd")
        "--agents"
        (import ./agents-json.nix { inherit lib pkgs; })
        "--claude"
        (lib.getExe pkgs.claude-code)
        "--quota-device"
        "/dev/disk/by-label/${home.label}"
      ];
      RuntimeDirectory = "agentd";
      RuntimeDirectoryMode = "0755";
      OOMScoreAdjust = -1000;
    };
  };

  services.openssh = {
    openFirewall = false;
    listenAddresses = [
      {
        addr = "127.0.0.1";
        port = 22;
      }
    ];
  };

  networking = {
    useNetworkd = true;
    hosts."127.0.0.9" = [ "api.anthropic.com" ];
  };
  systemd.network.networks."10-lan" = {
    matchConfig.MACAddress = mac;
    networkConfig.DHCP = "yes";
  };

  environment.systemPackages = [ pkgs.quota ];

  system.stateVersion = "26.11";
}
