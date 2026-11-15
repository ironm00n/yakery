# His projects the accounts work in, each bound under `top` in his home's layout with its Claude memory dir (layout.nix),
# so no data moves; each account's home links its clones to them (home/shares.nix). `top` is root's, so no account owns
# any part of a mount path; what it may read or write inside is his ACLs' call, never a rule here, since an owner change
# on a mount point would land on his tree.
{
  config,
  lib,
  utils,
  ...
}:
let
  inherit (import ./layout.nix { inherit lib; })
    top
    projects
    memories
    memory
    repos
    worktree
    ;
  # the accounts' own, to traverse `top`, and his, to share the repos under it
  group = baseNameOf top;
  under = map (dir: "${top}/${dir}");
  # each shared repo and the worktree dir: what any member makes inside stays group shr's to write (setgid, default
  # ACL), which jj's lock files in a shared store need; their parents are root's, like `top`, so no mount path is a
  # member's
  shared = under (repos ++ [ worktree ]);
  parents = under (lib.remove "." (lib.unique (map dirOf repos)));
  him = "ironmoon";
  his = config.users.users.${him}.home;
  # a project's dir and its Claude memory dir, in a home's layout
  dirs = home: project: [
    "${home}/${project}"
    (memory home project)
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
  binds =
    lib.concatMap (
      project:
      lib.zipListsWith (where: dir: lib.nameValuePair where (bind dir)) (dirs top project) (
        dirs his project
      )
    ) projects
    ++ map (project: lib.nameValuePair (memory top project) (bind (memory his project))) memories;
in
{
  options.agent.shares = lib.mkOption {
    readOnly = true;
    default = lib.listToAttrs binds;
    description = "Each project and its Claude memory dir, and the memory dir alone of each in `memories`, at their paths in his home with ${top} for the home, as fileSystems entries.";
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
    users.groups.${group}.members = builtins.attrNames (import ./accounts.nix) ++ [ him ];
    # his named entries let in the sessions he started before joining the group, which keep their old groups
    systemd.tmpfiles.rules = [
      "a+ ${top} - - - - user:${him}:x"
    ]
    ++ map (dir: "d ${dir} 0750 root ${group} -") parents
    ++ map (dir: "a+ ${dir} - - - - user:${him}:rx") parents
    ++ lib.concatMap (dir: [
      "d ${dir} 2770 root ${group} -"
      "a+ ${dir} - - - - group:${group}:rwx,default:group:${group}:rwx,user:${him}:rwx,default:user:${him}:rwx"
    ]) shared;
    # git refuses a repo another uid owns ("dubious ownership"); `/*` covers every depth below
    programs.git = {
      enable = true;
      config.safe.directory = "${top}/*";
    };
    agent.groupsWithHim = [ group ];
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
      text = "install -d -m 0710 -o root -g ${group} ${top}";
    };
  };
}
