{
  config,
  lib,
  pkgs,
  utils,
  ...
}:
let
  agents = import ./agents.nix { inherit lib; };
  mac = "02:61:67:6e:74:01";
  apiAddress = "127.0.0.9";
  mib = gib: gib * 1024;
  bytes = gib: mib gib * 1024 * 1024;
  imagePath = name: "/var/lib/agents/vm/${name}.img";
  volume = name: mountPoint: sizeGiB: mkfsArgs: {
    image = imagePath name;
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
  buildDir = "${overlayDir}/build";
  home = volume "home" "/home" 640 [
    "-O"
    "quota"
  ];
  # The key agentd shares with the host, which writes this image once a boot. A raw device in the
  # guest is readable by root alone.
  key = {
    image = imagePath "agentd-key";
    serial = "agentd-key";
    readOnly = true;
    autoCreate = false;
    mountPoint = null;
    size = 1;
  };
  keyDevice = "/dev/disk/by-id/virtio-${key.serial}";
  keyDeviceUnit = "${utils.escapeSystemdPath keyDevice}.device";
in
{
  imports = [
    ./agent-mail.nix
    (import ./agentd.nix {
      key = keyDevice;
      keyUnit = keyDeviceUnit;
      quotaDevice = "/dev/disk/by-label/${home.label}";
      sessions = config.microvm.vcpu;
      inherit apiAddress;
      claude = lib.getExe pkgs.claude-code;
    })
  ];

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
      key
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

  systemd.tmpfiles.settings."10-agents-shared"."/home/shared" = {
    d = {
      group = "agents";
      mode = "2775";
    };
    "a+".argument = "group:agents:rwx,default:group:agents:rwx,default:mask::rwx";
  };

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
    hosts.${apiAddress} = [ "api.anthropic.com" ];
  };
  systemd.network.networks."10-lan" = {
    matchConfig.MACAddress = mac;
    networkConfig.DHCP = "yes";
  };

  environment.systemPackages = with pkgs; [
    quota
    fd
    ripgrep
    moreutils
    poppler-utils
    pandoc
    jq
    gh
    git
    gnupg
    openssh
    file
    unzip
    zip
    coreutils-full
    findutils
    diffutils
    gawk
    gnused
    gnugrep
    binutils
    gnumake
    just
    tree
  ];
  environment.etc."claude-code/CLAUDE.md".text = lib.concatMapStringsSep "\n" builtins.readFile [
    ./CLAUDE.md
    ../../../users/ironmoon/bundles/dev/claude/CLAUDE.md
  ];
  environment.etc."claude-code/managed-settings.json".text = builtins.toJSON {
    model = "claude-opus-5-5";
    effortLevel = "medium";
  };

  system.stateVersion = "26.11";
}
