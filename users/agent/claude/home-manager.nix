# Its Claude Code settings are Owen's module as the agent's, and its sessions' supervisor is his unit, from the
# claude-tap its profile holds.
{ config, ... }:
let
  inherit (config.lib.file) mkOutOfStoreSymlink;
  home = config.home.homeDirectory;
  # notes' claude-tap package ships the unit his own account links from its checkout
  sessions = mkOutOfStoreSymlink "${home}/.nix-profile/share/systemd/user/claude-sessions.service";
in
{
  imports = [ ../../ironmoon/bundles/dev/claude ];
  bundles.dev.claude = {
    enable = true;
    account = "agent";
  };
  # the account's own secrets, where his sessions' habits and settings look for them
  home.file.".claude/secrets".source =
    mkOutOfStoreSymlink "${home}/${(import ../account.nix).secrets}";
  xdg.configFile = {
    "systemd/user/claude-sessions.service".source = sessions;
    "systemd/user/default.target.wants/claude-sessions.service".source = sessions;
  };
}
