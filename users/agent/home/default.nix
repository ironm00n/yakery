# Each account's home, which the account deploys itself with home-manager from homeConfigurations, so a system
# switch never touches it and updating it asks for no root.
account: {
  imports = [
    ./claude.nix
    ./identity.nix
    ./forges.nix
    ./xb.nix
    ./shares.nix
  ];
  _module.args = { inherit account; };
  home = {
    username = account.name;
    homeDirectory = "/home/${account.name}";
    # what its home had as part of desktop's system
    stateVersion = "24.05";
  };
}
