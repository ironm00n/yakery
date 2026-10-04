{ lib, ... }:
{
  disko.devices = {
    disk = {
      nvme = {
        type = "disk";
        device = lib.mkDefault "/dev/disk/by-id/nvme-Samsung_SSD_970_EVO_Plus_2TB_S59CNM0R849183D";
        content = {
          type = "gpt";
          partitions = {
            esp = {
              size = "2G";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [
                  "fmask=0077"
                  "dmask=0077"
                ];
              };
            };
            luks = {
              size = "100%";
              content = {
                type = "luks";
                name = "cryptroot";
                passwordFile = "/tmp/root.key";
                settings = {
                  allowDiscards = true;
                  bypassWorkqueues = true;
                  crypttabExtraOpts = [ "tpm2-device=auto" ];
                };
                content = {
                  type = "lvm_pv";
                  vg = "desktop";
                };
              };
            };
          };
        };
      };
      agents = {
        type = "disk";
        device = lib.mkDefault "/dev/disk/by-id/ata-WDC_WDS100T2B0B_20097Z444814";
        content = {
          type = "gpt";
          partitions.luks = {
            size = "100%";
            content = {
              type = "luks";
              name = "agents";
              initrdUnlock = false;
              settings.keyFile = "/tmp/agents.key";
              content = {
                type = "filesystem";
                format = "ext4";
                mountpoint = "/var/lib/agents";
                mountOptions = [ "nofail" ];
              };
            };
          };
        };
      };
    };
    lvm_vg.desktop = {
      type = "lvm_vg";
      lvs = {
        swap = {
          size = lib.mkDefault "48G";
          content = {
            type = "swap";
            resumeDevice = true;
          };
        };
        root = {
          size = "100%FREE";
          content = {
            type = "filesystem";
            format = "ext4";
            mountpoint = "/";
          };
        };
      };
    };
  };
}
