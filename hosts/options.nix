{ lib, ... }:
let
  inherit (lib) mkOption types;
in
{
  options.host = {
    id = mkOption {
      type = types.str;
      description = "Unique identifier for this machine";
    };

    hostname = mkOption {
      type = types.str;
      description = "Hostname of this machine";
      default = "nixos";
    };

    laptop = mkOption {
      type = types.bool;
      default = false;
      description = "Is a laptop";
    };

    fingerprint = mkOption {
      type = types.bool;
      default = false;
      description = "Supports fingerprint authentication";
    };

    kde = mkOption {
      type = types.bool;
      default = false;
      description = "KDE Plasma is used";
    };

    hyprland = mkOption {
      type = types.bool;
      default = false;
      description = "Hyprland is used";
    };

    nvidia = mkOption {
      type = types.bool;
      default = false;
      description = "shitty graphics card";
    };

    additional-user-pkgs = mkOption {
      type = types.listOf types.package;
      default = [ ];
      description = "Additional user packages specific to the host.";
    };

    out-of-store-symlinks = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to use impure out of store symlinks for some dotfiles.";
    };

    home-manager-nixos = mkOption {
      type = types.bool;
      default = true;
      description = "Use the NixOS home-manager module.";
    };
  };
}
