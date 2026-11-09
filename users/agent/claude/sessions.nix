# What each account's user manager hands every Claude Code session it starts.
{
  systemd.services = builtins.listToAttrs (
    map (
      { uid, ... }:
      {
        name = "user@${toString uid}";
        value = {
          overrideStrategy = "asDropin";
          environment = {
            # each session's own scope, which the sessions' spawn reads
            CLAUDE_SESSION_MEMORY_MAX = "2G";
            # it has no Claude login: its tap sends every turn to the pool under the pool's token in place of this one
            CLAUDE_CODE_OAUTH_TOKEN = "the-tap-replaces-this";
          };
        };
      }
    ) (builtins.attrValues (import ../accounts.nix))
  );
}
