{ lib, ... }:
{
  imports = [
    ./hyprland.nix
    ./kde.nix
  ];

  options.specialisation = lib.mkOption {
    type = lib.types.attrsOf (
      lib.types.submodule (
        { name, ... }:
        {
          config.configuration.environment.etc.specialisation.text = name;
        }
      )
    );
  };
}
