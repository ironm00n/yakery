{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) getExe mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.hyprland;
  hl = config.wayland.windowManager.hyprland;
  vmClip = import ../../../../nix/vms/hackathon-vm-clip.nix { inherit pkgs; };
  hyprlandFennel = pkgs.callPackage ../../../../packages/hyprland-fennel { };
in
{
  options.bundles.hyprland = {
    enable = mkEnableOption "hyprland";
  };

  config = mkIf cfg.enable {
    # hyprland.fnl is found before hyprland.lua, so anything home-manager would generate
    # is silently thrown away
    assertions = [
      {
        # mirrors `shouldGenerate` in home-manager's hyprland lib
        assertion =
          !hl.systemd.enable
          && hl.settings == { }
          && hl.plugins == [ ]
          && hl.extraConfig == ""
          && hl.extraLuaFiles == { }
          && hl.submaps == { };
        message = "bundles.hyprland: the config is hand-written hypr/*.fnl, so anything home-manager would generate is discarded; move it into the fennel files.";
      }
    ];

    wayland.windowManager.hyprland = import ./config.nix { inherit pkgs; };

    xdg.configFile = {
      "hypr/xdph.conf".text = lib.hm.generators.toHyprconf {
        attrs = import ./portal.nix { inherit pkgs; };
      };

      "hypr/hyprland.fnl".source = symlink ./hyprland.fnl;
      "hypr/conf".source = symlink ./conf;
      "hypr/lib".source = symlink ./lib;
      "hypr/hl-keys.fnl".source = "${hyprlandFennel}/macros/hl-keys.fnl";

      "hypr/nix.lua".text = "return ${
        lib.generators.toLua { } {
          clip-to-vm = getExe vmClip.push;
          clip-from-vm = getExe vmClip.pull;
          inherit (config.host) nvidia;
        }
      }";
    };
  };
}
