# Its Claude Code settings are Owen's module as the agent's.
{ osConfig, ... }:
{
  imports = [ ../ironmoon/bundles/dev/claude ];
  bundles.dev.claude = {
    enable = true;
    account = "agent";
  };
  home.stateVersion = osConfig.system.stateVersion;
}
