{
  config,
  lib,
  pkgs,
  pkgs-stable,
  inputs,

  ...
}:
let
  inherit (lib) mkEnableOption mkIf optionals;
  inherit (inputs) binary-ninja pwndbg;
  cfg = config.bundles.sec;
in
{
  options.bundles.sec = {
    enable = mkEnableOption "security";
    re = mkEnableOption "reverse engineering";
    ctf = mkEnableOption "CTF related tools and config";
    ld = mkEnableOption "nix-ld";
  };

  config = mkIf cfg.enable {
    environment.systemPackages =
      optionals cfg.ctf (
        with pkgs;
        [
          hashcat # FIXME: CL_PLATFORM_NOT_FOUND_KHR
          john # the ripper
          hash-identifier
          binwalk
          zsteg
          steghide

          pkgs-stable.sonic-visualiser
        ]
      )
      ++ optionals cfg.re (
        with pkgs;
        [
          gef

          binary-ninja.packages.${system}.binary-ninja-free-wayland
          pwndbg.packages.${system}.pwndbg
        ]
      );

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
