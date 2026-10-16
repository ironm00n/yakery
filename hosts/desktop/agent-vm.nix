{
  config,
  inputs,
  lib,
  pkgs,
  ...
}:
let
  imageDir = "/var/lib/agents/vm";
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
      ExecStartPre = [
        "+${pkgs.coreutils}/bin/install -d -o microvm -g ${config.users.users.microvm.group} -m 0700 ${imageDir}"
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
        (toString config.microvm.vms.agents.config.config.microvm.vsock.cid)
      ];
      DynamicUser = true;
      Restart = "always";
    };
  };
  networking.firewall.interfaces.nb-netbird.allowedTCPPorts = [ dispatchPort ];
}
