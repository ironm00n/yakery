{
  config,
  inputs,
  pkgs,
  ...
}:
let
  imageDir = "/var/lib/agents/vm";
in
{
  imports = [ inputs.microvm.nixosModules.host ];

  microvm.vms.agents = {
    config = ../../nix/vms/agents;
    autostart = false;
  };

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
}
