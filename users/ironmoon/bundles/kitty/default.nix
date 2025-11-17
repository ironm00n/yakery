# see https://sw.kovidgoyal.net/kitty/conf/
{
  config,
  lib,
  pkgs,
  my-utils,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.kitty;
  inherit (config.xdg) configHome;
in
{
  options.bundles.kitty = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable kitty.";
    };
  };

  config = mkIf cfg.enable {
    home.packages = with pkgs; [
      eww
      hyprland-workspaces
    ];

    # Adapted from hm module
    programs.zsh.initContent = /* zsh */ ''
      if test -n "$KITTY_INSTALLATION_DIR"; then
        export KITTY_SHELL_INTEGRATION="no-rc"
        autoload -Uz -- "$KITTY_INSTALLATION_DIR"/shell-integration/zsh/kitty-integration
        kitty-integration
        unfunction kitty-integration
      fi
    '';

    xdg.configFile."${configHome}/kitty/kitty.conf".source = symlink ./kitty.conf;
  };
}
