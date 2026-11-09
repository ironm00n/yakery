# Its Claude Code settings are Owen's module as the agent's, and its sessions' supervisor is his unit, from the
# claude-tap its profile holds.
{ config, ... }:
let
  # notes' claude-tap package ships the unit his own account links from its checkout
  sessions = config.lib.file.mkOutOfStoreSymlink "${config.home.homeDirectory}/.nix-profile/share/systemd/user/claude-sessions.service";
in
{
  imports = [ ../../ironmoon/bundles/dev/claude ];
  bundles.dev.claude = {
    enable = true;
    account = "agent";
  };
  xdg.configFile = {
    "systemd/user/claude-sessions.service".source = sessions;
    "systemd/user/default.target.wants/claude-sessions.service".source = sessions;
  };
}
