{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.displaylink;

  # nixpkgs' 1.14.15 has no linux >= 7.2 support, and its prePatch targets the
  # distro detection 1.15.0 dropped; https://github.com/NixOS/nixpkgs/pull/555981
  evdi = config.boot.kernelPackages.evdi.overrideAttrs (
    finalAttrs: prevAttrs: {
      version = "1.15.0";
      src = pkgs.fetchFromGitHub {
        owner = "DisplayLink";
        repo = "evdi";
        tag = "v${finalAttrs.version}";
        hash = "sha256-CXF7PvmrPjjNoWXbWxEkFE/Sw4bO6YqDplPwF/OxhB0=";
      };
      prePatch = "";
      # the 1.15.0 feature probes miss KBUILD_MODNAME, which CONFIG_MEM_ALLOC_PROFILING
      # needs; https://github.com/DisplayLink/evdi/pull/592
      patches = prevAttrs.patches or [ ] ++ [ ./conftest-kbuild-modname.patch ];
    }
  );

  displaylink = pkgs.displaylink.override { inherit evdi; };
in
{
  options.bundles.displaylink = {
    enable = mkEnableOption "DisplayLink drivers";
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [
      displaylink
    ];

    boot = {
      extraModulePackages = [ evdi ];
      initrd = {
        kernelModules = [
          "evdi"
        ];
      };
    };

    systemd.services.displaylink-server = {
      enable = true;
      requires = [ "systemd-udevd.service" ];
      after = [ "systemd-udevd.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "simple";
        ExecStart = "${displaylink}/bin/DisplayLinkManager";
        User = "root";
        Group = "root";
        # Environment = [ "DISPLAY=:0" ];
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
