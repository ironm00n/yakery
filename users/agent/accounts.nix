# The accounts agents run as, by name, each with its uid (its group's gid too), its own anthropic-tap's loopback port
# and whether it is privileged (privileged.nix). Every module here holds for each of them.
builtins.mapAttrs
  (
    name: account:
    {
      privileged = false;
    }
    // account
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
      privileged = true;
    };
  }
