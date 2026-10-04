{
  config,
  inputs,
  my-lib,
  pkgs,
  ...
}:
let
  agentsKey = my-lib.sops.mkSecrets {
    inherit config;
    sopsFile = inputs.secrets.lib.luks.desktop-agents;
    prefix = "luks-agents";
  } [ "key" ];
in
{
  boot.loader = {
    efi.canTouchEfiVariables = true;
    limine = {
      enable = true;
      maxGenerations = 10;
      extraInstallCommands = ''
        ${pkgs.coreutils}/bin/install -D -m 0644 \
          ${config.boot.loader.efi.efiSysMountPoint}/EFI/limine/BOOTX64.EFI \
          ${config.boot.loader.efi.efiSysMountPoint}/EFI/BOOT/BOOTX64.EFI
      '';
      secureBoot.enable = true;
    };
  };

  boot.initrd = {
    availableKernelModules = [ "r8169" ];
    systemd = {
      enable = true;
      # a passphrase sent over initrd ssh can arrive after the 90s default drops to emergency mode
      settings.Manager.DefaultDeviceTimeoutSec = "infinity";
      network = {
        enable = true;
        networks."10-ether" = {
          matchConfig.Type = "ether";
          networkConfig.DHCP = "ipv4";
          dhcpV4Config.ClientIdentifier = "mac";
        };
      };
    };
    network.ssh = {
      enable = true;
      port = 2222;
      hostKeys = [ "/etc/ssh/initrd_ssh_host_ed25519_key" ];
    };
  };

  security.tpm2.enable = true;

  sops.secrets = agentsKey.secrets;
  environment.etc.crypttab.text = ''
    agents /dev/disk/by-partlabel/disk-agents-luks ${agentsKey.get-path "key"} luks,discard,nofail
  '';

  environment.systemPackages = with pkgs; [
    sbctl
    tpm2-tools
  ];
}
