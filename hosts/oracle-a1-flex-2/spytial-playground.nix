{
  pkgs,
  my-lib,
  ...
}:
let
  user = "spytial-playground";
  base = "/var/lib/spytial-playground";
  port = 8931;
  nodejs = pkgs.nodejs_24;
in
{
  users.groups.${user} = { };

  users.users.${user} = {
    isSystemUser = true;
    group = user;
    home = base;
  };

  systemd.tmpfiles.rules = [ "d ${base} 0755 ${user} ${user} -" ];

  # The payload (lean4web checkout + built spytial-cslib) is provisioned by
  # playground/deploy/provision-vps.sh, which runs on this host as ${user} and
  # needs the same toolchain the service does.
  environment.systemPackages = [
    nodejs
    pkgs.pnpm
    pkgs.elan
    pkgs.bubblewrap
  ];

  systemd.services.spytial-playground = {
    description = "spytial-cslib lean4web playground";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];

    # Skip (not fail) until the payload is provisioned.
    unitConfig.ConditionPathExists = "${base}/lean4web/client/dist";

    # The server spawns server/bubblewrap.sh (bash, realpath) per session,
    # which resolves the toolchain through elan's shims before entering bwrap;
    # its bwrap availability probe shells out to `which`, and the pre-container
    # `lake env` needs git or lake deletes + re-clones dependencies.
    path = [
      nodejs
      pkgs.elan
      pkgs.bubblewrap
      pkgs.bash
      pkgs.coreutils
      pkgs.which
      pkgs.git
    ];

    environment = {
      ELAN_HOME = "${base}/elan";
      PORT = toString port;
      PROJECTS_BASE_PATH = "../Projects";
      VITE_COLLAB = "false";
    };

    serviceConfig =
      removeAttrs my-lib.systemd.hardening.network [
        # bwrap needs user/mount/pid namespaces, mount(2) & co, and an unmasked
        # /proc to mount a fresh procfs inside the sandbox.
        "RestrictNamespaces"
        "SystemCallFilter"
        "ProtectProc"
        "ProcSubset"
        "ProtectKernelTunables"
        "ProtectKernelLogs"
        # both leave overmounts on /proc (kcore, hostname), same EPERM
        "PrivateDevices"
        "ProtectHostname"
      ]
      // {
        Type = "simple";
        User = user;
        Group = user;

        WorkingDirectory = "${base}/lean4web";
        ExecStart = "${nodejs}/bin/npm run prod";

        Restart = "always";
        RestartSec = 2;

        ReadWritePaths = [ base ];
        MemoryDenyWriteExecute = false; # V8 JIT
        # bwrap sets up loopback in the session netns over a netlink socket
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
          "AF_NETLINK"
        ];
      };
  };

  bundles.reverse-proxy = {
    enable = true;
    openFirewall = true;
    acme-email = "owen@duckham.dev";
    hosts = {
      # Reachable over the overlay now; the public name additionally needs an
      # A record -> this box and OCI ingress for 80/443.
      "oracle-a1-flex-2.h.im.exposed" = {
        inherit port;
        websockets = true;
        tls = false;
      };
      "spytial-cslib.ironmoon.dev" = {
        inherit port;
        websockets = true;
      };
    };
  };
}
