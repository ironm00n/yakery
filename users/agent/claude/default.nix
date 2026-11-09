# Claude Code on each account: its own tap, Owen's settings as the agent's, and its sessions' supervisor.
{
  imports = [
    ./tap.nix
    ./sessions.nix
  ];
  home-manager.users = builtins.mapAttrs (_: account: import ./home-manager.nix account) (
    import ../accounts.nix
  );
}
