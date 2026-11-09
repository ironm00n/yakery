# The accounts agents run as when Owen starts them there (accounts.nix), whatever the tool; each tool is a layer of its
# own beside them.
{ config, ... }:
{
  imports = [
    ./user.nix
    ./fence.nix
    ./identity.nix
    ./forges.nix
    ./xb.nix
    ./shares.nix
    ./claude
  ];
  home-manager.users = builtins.mapAttrs (_: _: { home.stateVersion = config.system.stateVersion; }) (
    import ./accounts.nix
  );
}
