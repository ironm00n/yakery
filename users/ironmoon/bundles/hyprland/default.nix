{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.hyprland;
  hl = config.wayland.windowManager.hyprland;
  # hyprland prefers hyprland.lua over hyprland.conf, so the hand-written lua may
  # only be placed when it is actually the configured format
  lua-config = hl.configType == "lua";

  lua-symlink  = path: mkIf lua-config { source = symlink path; };

in
{
  options.bundles.hyprland = {
    enable = mkEnableOption "hyprland";
  };

  config = mkIf cfg.enable {
    # a hand-written hyprland.lua overrides home-manager's generated one without any
    # conflict error, so anything that would be generated is silently thrown away
    assertions = [
      {
        # mirrors `shouldGenerate` in home-manager's hyprland lib
        assertion =
          lua-config
          -> (
            !hl.systemd.enable
            && hl.settings == { }
            && hl.plugins == [ ]
            && hl.extraConfig == ""
            && hl.extraLuaFiles == { }
            && hl.submaps == { }
          );
        message = "bundles.hyprland: hypr/hyprland.lua is hand-written, so anything home-manager would generate into it is discarded; move it into the lua file.";
      }
    ];

    wayland.windowManager.hyprland = import ./config.nix { inherit config lib pkgs; };

    xdg.configFile = {
      "hypr/xdph.conf".text = lib.hm.generators.toHyprconf {
        attrs = import ./portal.nix { inherit pkgs; };
      };

      "hypr/hyprland.lua" = lua-symlink ./hyprland.lua;
      "hypr/utils.lua" = lua-symlink ./utils.lua;
      "hypr/conf" =  lua-symlink ./conf;
    };
  };
}
