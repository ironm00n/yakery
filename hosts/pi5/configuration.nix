{
  inputs,
  lib,
  pkgs,
  ...
}:

{
  imports = [
    inputs.nixos-hardware.nixosModules.raspberry-pi-5
    "${inputs.nixpkgs}/nixos/modules/installer/sd-card/sd-image-aarch64.nix"
    ../common/server
  ];

  # The profile defaults to a downstream rpi kernel that nobody caches -- not
  # cache.nixos.org, nix-community, nixos-raspberrypi or raspberry-pi-nix (all
  # 404 on its hash), so every nixpkgs bump would mean recompiling a kernel on
  # an Ampere box. Mainline is substitutable and carries the pi5 dtb, MACB=y for
  # the RP1 NIC, MISC_RP1/PINCTRL_RP1/COMMON_CLK_RP1 and MMC_SDHCI_BRCMSTB=y.
  # kernelPackages is mkDefault in the profile, and the profile's initrd list
  # switches itself to the rp1_pci/pinctrl-rp1 branch when it sees mainline.
  boot.kernelPackages = pkgs.linuxPackages;

  # uboot.enable defaults this to false, which hands the kernel the *firmware's*
  # dtb. Right for the vendor kernel, fatal for mainline: rp1_pci.c does
  # `of_find_node_by_name(NULL, "rp1_nexus")`, and the vendor tree names that
  # node plain `rp1` with no PCI reg, so the driver exits -EINVAL and the whole
  # of RP1 -- usb as much as ethernet -- never comes up. Observed on hardware
  # 2026-09-05: clean boot to userspace, no NIC, no link lights.
  # true makes extlinux emit FDTDIR so u-boot loads the kernel's own dtb; a miss
  # there falls back to the firmware dtb rather than booting with none.
  boot.loader.generic-extlinux-compatible.useGenerationDeviceTree = true;

  # nixos-hardware mkForce-replaces sdImage.populateFirmwareCommands, so the
  # sd-image module's u-boot.bin copy and its `kernel=u-boot.bin` are discarded
  # rather than merged. The replacement ships u-boot only when this is set, and
  # it defaults off: without it the firmware partition has no bootloader, the
  # EEPROM loads the dtb, finds no kernel and halts. nixos-hardware#1928.
  hardware.raspberry-pi.firmware.uboot.enable = true;

  # sd-image.nix enables enableAllHardware, which drags a long generic module
  # list into the initrd; modules-shrunk then fails on anything the kernel does
  # not build. nixos-hardware#2006. enableAllHardware was also what pulled in
  # redistributable firmware, so that has to come back explicitly.
  hardware.enableAllHardware = lib.mkForce false;
  hardware.enableRedistributableFirmware = true;

  # systemd initrd is on here, and it adds tpm-tis + tpm-crb by default. There
  # is no TPM on a Pi at all, and a missing module is a hard build failure.
  # Second half of nixos-hardware#2006.
  boot.initrd.systemd.tpm2.enable = false;

  # Unverified: nixpkgs#70230 reports a pi5 not booting headless without this,
  # and I could not reproduce the mechanism in u-boot's source. Kept because it
  # costs nothing and this machine has no console once it is in place.
  hardware.raspberry-pi.configtxt.settings.all.hdmi_force_hotplug = true;

  bundles.vpn.netbird = {
    enable = true;
    useSetupKey = true;

    # Forwards for the rest of the LAN, not just for itself.
    routing = "server";
  };

  # The image is assembled on a remote aarch64 builder and pulled back over a
  # ~22 MiB/s link, so the transfer dominates, not local CPU: zstd turns a 5.1GB
  # copy into roughly 1.5GB. Decompress on the way to the card.
  sdImage.compressImage = true;

  # The firmware module copies every start*.elf plus the dtbs and ~371
  # overlays, landing near 27MB inside the 30MiB default. It fits, but not by
  # enough to bet a build on.
  sdImage.firmwareSize = 64;

  # base.nix would otherwise build a zfs module for a machine whose only disk
  # is an SD card. mkForce is belt-and-braces: base.nix uses mkDefault today.
  boot.supportedFilesystems.zfs = lib.mkForce false;

  system.stateVersion = "26.11";
}
