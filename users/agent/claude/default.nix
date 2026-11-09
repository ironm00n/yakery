# Claude Code on the agent account: its own tap, Owen's settings as the agent's, and its sessions' supervisor.
{
  imports = [
    ./tap.nix
    ./sessions.nix
  ];
  home-manager.users.${(import ../account.nix).name} = ./home-manager.nix;
}
