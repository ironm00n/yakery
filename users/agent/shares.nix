# His projects the account works in, each bound under `top` in his home's layout with its Claude memory dir, so no data
# moves. `top` is root's, so the account owns no part of a mount path; what it may read or write inside is his ACLs'
# call, never a rule here, since an owner change on a mount point would land on his tree.
{
  config,
  lib,
  utils,
  ...
}:
let
  inherit (import ./account.nix) name;
  top = "/shr";
  # checkouts under his home
  projects = [ "workspace/redox" ];
  his = config.users.users.ironmoon.home;
  # Claude's name for a project's dir under ~/.claude/projects
  slug = lib.stringAsChars (c: if builtins.match "[a-zA-Z0-9]" c == null then "-" else c);
  # a project's dir and its Claude memory dir, in a home's layout
  dirs =
    home: project:
    let
      dir = "${home}/${project}";
    in
    [
      dir
      "${home}/.claude/projects/${slug dir}/memory"
    ];
  bind = device: {
    inherit device;
    fsType = "none";
    options = [
      "bind"
      # a mount the account makes in a share stays out of his tree
      "slave"
      "nofail"
    ];
  };
  binds = lib.concatMap (
    project: lib.zipListsWith (where: dir: lib.nameValuePair where (bind dir)) (dirs top project) (dirs his project)
  ) projects;
in
{
  options.agent.shares = lib.mkOption {
    readOnly = true;
    default = lib.listToAttrs binds;
    description = "Each project and its Claude memory dir, at their paths in his home with ${top} for the home, as fileSystems entries.";
  };

  config = {
    assertions = [
      {
        assertion =
          let
            wheres = lib.attrNames config.agent.shares;
          in
          lib.all (a: lib.all (b: !lib.hasPrefix "${a}/" b) wheres) wheres;
        message = "users/agent/shares.nix: a share lies inside another, which would put its mount point in his tree";
      }
    ];
    fileSystems = config.agent.shares;
    # systemd makes a missing bind source as root, which would put a root-owned dir in his home
    systemd.units = lib.mapAttrs' (
      where: bind:
      lib.nameValuePair "${utils.escapeSystemdPath where}.mount" {
        overrideStrategy = "asDropin";
        text = "[Unit]\nConditionPathIsDirectory=${bind.device}\n";
      }
    ) config.agent.shares;
    # before systemd mounts anything, at boot and at each switch
    system.activationScripts.agent-shares = {
      deps = [ "users" ];
      text = "install -d -m 0710 -o root -g ${name} ${top}";
    };
  };
}
