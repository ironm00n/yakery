# What the account's user manager hands every Claude Code session it starts.
let
  inherit (import ../account.nix) uid;
in
{
  systemd.services."user@${toString uid}" = {
    overrideStrategy = "asDropin";
    environment = {
      # each session's own scope, which the sessions' spawn reads
      CLAUDE_SESSION_MEMORY_MAX = "2G";
      # it has no Claude login: its tap sends every turn to the pool under the pool's token in place of this one
      CLAUDE_CODE_OAUTH_TOKEN = "the-tap-replaces-this";
    };
  };
}
