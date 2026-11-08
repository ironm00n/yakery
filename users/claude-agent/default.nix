{
  imports = [
    ./user.nix
    ./tap.nix
    ./identity.nix
  ];
  home-manager.users.claude-agent = ./home-manager.nix;
}
