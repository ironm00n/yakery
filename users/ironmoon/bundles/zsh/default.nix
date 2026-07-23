{
  config,
  lib,
  my-utils,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.zsh;
in
{
  options.bundles.zsh = {
    enable = mkEnableOption "zsh";
  };

  config = mkIf cfg.enable {
    home.file.".p10k.zsh".source = symlink ./.p10k.zsh;

    # some tools will put random shit and here, which will break stuff
    home.file.".zprofile".text = "";
    home.file.".profile".text = "";

    programs.zsh = import ./program.nix { inherit lib config pkgs; };
  };
}
