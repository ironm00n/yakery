{
  config,
  inputs,
  lib,
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
  agentd = pkgs.callPackage ../../packages/agentd/package.nix { };
  dispatchPort = 7310;
in
{
  imports = [ inputs.microvm.nixosModules.host ];

  microvm.vms.agents.config = ../../nix/vms/agents;

  environment.systemPackages = [ agentd ];

  systemd.services."microvm@agents" = {
    unitConfig.RequiresMountsFor = [ imageDir ];
    serviceConfig = {
      CPUWeight = 20;
      # A kept overlay keeps GC's whiteouts over erofs paths that a later image ships again, and
      # the guest's nix database on its tmpfs root forgets what the overlay holds anyway. The runner
      # recreates the image empty; doing it here rather than in the guest avoids udev re-probing
      # the volume under its own fsck.
      ExecStartPre = [
        "+${pkgs.coreutils}/bin/install -d -o microvm -g ${config.users.users.microvm.group} -m 0700 ${imageDir}"
        "+${pkgs.coreutils}/bin/rm -f ${overlayImage}"
      ];
    };
  };

  networking.networkmanager.unmanaged = [ "interface-name:vm-agents" ];

  systemd.services.agent-dispatch = {
    description = "Wake agents when mail arrives for them";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe' agentd "agent-dispatch")
        "--listen"
        "[::]:${toString dispatchPort}"
        "--agents"
        (import ../../nix/vms/agents/agents-json.nix { inherit lib pkgs; })
        "--domain"
        "agent.ironmoon.dev"
        "--cid"
        (toString guest.vsock.cid)
      ];
      DynamicUser = true;
      Restart = "always";
    };
  };
  networking.firewall.interfaces.nb-netbird.allowedTCPPorts = [ dispatchPort ];
}
