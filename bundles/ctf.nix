{
  config,
  lib,
  pkgs,
  pkgs-stable,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  cfg = config.bundles.ctf;
in
{
  options.bundles.ctf = {
    enable = mkEnableOption "CTF related tools and config";
    ld = mkEnableOption "nix-ld for CTFs";
  };

  config = mkIf cfg.enable {
    environment.systemPackages = with pkgs; [
      hashcat # FIXME: CL_PLATFORM_NOT_FOUND_KHR
      john # the ripper
      hash-identifier
      binwalk
      zsteg
      steghide

      pkgs-stable.sonic-visualiser
    ];

    # WARNING nix-ld: this should only be used for hacky situations such as CTFs
    # otherwise this negates the benefits of nix
    programs.nix-ld = {
      enable = cfg.ld;
      libraries = with pkgs; [
        # Add any missing dynamic libraries for unpackaged programs
        # here, NOT in environment.systemPackages

        glib
        nss
        nspr
        dbus
        atk
        # atk-bridge
        at-spi2-atk
        libdrm
        gtk3
        pango
        cairo
        libx11
        xorg.libXcomposite
        xorg.libXdamage
        libxext
        xorg.libXfixes
        xorg.libXrandr
        # gbm
        mesa
        expat
        xorg.libxcb
        libxkbcommon
        alsa-lib
        # at-spi2-core
      ];
    };
  };
}
