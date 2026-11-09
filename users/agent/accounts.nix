# The accounts agents run as, by name, each with its uid (its group's gid too) and its own anthropic-tap's loopback
# port. Every module here holds for each of them.
builtins.mapAttrs
  (
    name: account:
    account
    // {
      inherit name;
      # its credentials, under its home: tokens Owen copies in, never in the store
      secrets = ".secrets";
    }
  )
  {
    agent = {
      uid = 1500;
      tapPort = 8891;
    };
    # TODO: the name is Owen's pick
    agent-privileged = {
      uid = 1501;
      tapPort = 8892;
    };
  }
