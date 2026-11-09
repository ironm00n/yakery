# The account agents run as when Owen starts them there, whatever the tool; each tool is a layer of its own beside it.
{ config, ... }:
let
  inherit (import ./account.nix) name;
in
{
  imports = [
    ./user.nix
    ./fence.nix
    ./identity.nix
    ./claude
  ];
  home-manager.users.${name}.home.stateVersion = config.system.stateVersion;
}
