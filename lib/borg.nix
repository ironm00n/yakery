{ lib }:
let
  inherit (lib) mkOption types;
in
{
  storagebox = {
    host = "u668784.your-storagebox.de";
    user = "u668784";
    port = 23;
    hostPublicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIICf9svRenC/PLKIL9nk6K/pxQgoiFC41wTNvoIncOxs";
    remotePath = "borg-1.4";
  };

  /**
    Options naming an SSH host that runs `borg serve`.
  */
  targetOptions = {
    host = mkOption {
      type = types.str;
      description = "SSH host running `borg serve`.";
    };

    user = mkOption {
      type = types.str;
      description = "SSH user on the target.";
    };

    port = mkOption {
      type = types.port;
      default = 22;
      description = "SSH port on the target.";
    };

    hostPublicKey = mkOption {
      type = types.str;
      description = "Pinned SSH host key of the target.";
    };

    remotePath = mkOption {
      type = types.str;
      default = "borg";
      description = "borg executable on the target.";
    };
  };

  /**
    The name ssh looks a target up by in known_hosts.
  */
  knownHostName =
    target: if target.port == 22 then target.host else "[${target.host}]:${toString target.port}";
}
