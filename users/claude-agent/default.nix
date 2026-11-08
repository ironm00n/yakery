{
  imports = [
    ./user.nix
    ./tap.nix
  ];
  home-manager.users.claude-agent = ./home-manager.nix;
}
