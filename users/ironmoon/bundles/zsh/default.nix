{
  config,
  lib,
  my-utils,
  pkgs,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  cfg = config.bundles.zsh;
in
{
  options.bundles.zsh = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable zsh config.";
    };
  };

  config = mkIf cfg.enable {
    # home.file.".p10k.zsh".source = symlink ./.p10k.zsh;
    home.file.".p10k.zsh".source = symlink "ironmoon/bundles/zsh/.p10k.zsh";

    programs.zsh = import ./program.nix { inherit lib config pkgs; };
  };
}
