# The accounts agents run as when Owen starts them there (accounts.nix), whatever the tool; each tool is a layer of its
# own beside them. Only what needs root is here: each account's home is its own (home/, homeConfigurations).
{
  imports = [
    ./user.nix
    ./privileged.nix
    ./fence.nix
    ./identity.nix
    ./xb.nix
    ./shares.nix
    ./claude
  ];
}
