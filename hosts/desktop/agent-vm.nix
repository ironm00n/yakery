{
  config,
  inputs,
  lib,
  my-lib,
  pkgs,
  ...
}:
let
  guest = config.microvm.vms.agents.config.config.microvm;
  overlayImage =
    (lib.findFirst (
      volume: volume.mountPoint == guest.writableStoreOverlay
    ) (throw "the agent VM has no volume for its store overlay") guest.volumes).image;
  imageDir = dirOf overlayImage;
  keyImage =
    (lib.findFirst (
      volume: volume.serial == "agentd-key"
    ) (throw "the agent VM has no key volume") guest.volumes).image;
  # The host's copy of the key agentd holds: the gateway and agentctl run as Owen
  keyFile = "/var/lib/agents/control.key";
  agentd = pkgs.callPackage ../../packages/agentd/package.nix { };
  agentctl = pkgs.writeShellScriptBin "agentctl" ''
    exec ${lib.getExe' agentd "agentctl"} --key ${keyFile} "$@"
  '';
  dispatchPort = 7310;
in
{
  imports = [ inputs.microvm.nixosModules.host ];

  microvm.vms.agents.config = {
    imports = [ ../../nix/vms/agents ];
    nixpkgs.flake.source = config.nixpkgs.flake.source;
  };

  environment.systemPackages = [ agentctl ];

  # The image goes in a directory microvm owns, so it is written as microvm: root would follow a
  # link planted there.
  systemd.services.agents-key = {
    description = "The key the agent VM's daemon and the host share";
    unitConfig.RequiresMountsFor = [ imageDir ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      UMask = "0077";
    };
    path = [
      pkgs.coreutils
      pkgs.util-linux
    ];
    script = ''
      set -o pipefail
      install -d -o microvm -g ${config.users.users.microvm.group} -m 0700 ${imageDir}
      if [ ! -s ${keyFile} ]; then
        head -c 32 /dev/urandom > ${keyFile}.new
        mv ${keyFile}.new ${keyFile}
      fi
      chown ${config.users.users.ironmoon.name}:root ${keyFile}
      chmod 0400 ${keyFile}
      # a virtio disk is whole sectors
      { cat ${keyFile}; head -c 480 /dev/zero; } |
        setpriv --reuid=microvm --regid=${config.users.users.microvm.group} --clear-groups \
          dd of=${keyImage} status=none
    '';
  };

  systemd.services."microvm@agents" = {
    requires = [ "agents-key.service" ];
    after = [ "agents-key.service" ];
    unitConfig.RequiresMountsFor = [ imageDir ];
    serviceConfig = {
      CPUWeight = 20;
      # A kept overlay keeps GC's whiteouts over erofs paths that a later image ships again, and
      # the guest's nix database on its tmpfs root forgets what the overlay holds anyway. The runner
      # recreates the image empty; doing it here rather than in the guest avoids udev re-probing
      # the volume under its own fsck.
      ExecStartPre = [ "+${pkgs.coreutils}/bin/rm -f ${overlayImage}" ];
    };
  };

  networking.networkmanager.unmanaged = [ "interface-name:vm-agents" ];

  systemd.services.agent-dispatch = {
    description = "Wake agents when mail arrives for them";
    wantedBy = [ "multi-user.target" ];
    requires = [ "agents-key.service" ];
    after = [ "agents-key.service" ];
    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe' agentd "agent-dispatch")
        "--listen"
        "[::]:${toString dispatchPort}"
        "--allow-from"
        my-lib.netbird.peers.hetzner-cx33-1.v4
        "--allow-from"
        my-lib.netbird.peers.hetzner-cx33-1.v6
        "--agents"
        (import ../../nix/vms/agents/agents-json.nix { inherit lib pkgs; })
        "--domain"
        "agent.ironmoon.dev"
        "--cid"
        (toString guest.vsock.cid)
        "--key"
        "%d/key"
      ];
      LoadCredential = [ "key:${keyFile}" ];
      DynamicUser = true;
      Restart = "always";
    };
  };
  networking.firewall.interfaces.nb-netbird.allowedTCPPorts = [ dispatchPort ];
}
