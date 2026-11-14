# What the system and each account's home agree on: the shares under `top`, each project's Claude memory dir in a
# home's layout, and the xb client the privileged account places.
{ lib }:
rec {
  top = "/shr";
  # checkouts under his home, each bound with its Claude memory dir
  projects = [ "workspace/redox" ];
  # projects whose Claude memory dir alone is bound: the accounts work in clones of their own at the same path under
  # their homes, push from there, and the memory stays one dir
  memories = [
    "workspace/claude"
    "workspace/autograder"
  ];
  # Claude's name for a project's dir under ~/.claude/projects
  slug = lib.stringAsChars (c: if builtins.match "[a-zA-Z0-9]" c == null then "-" else c);
  memory = home: project: "${home}/.claude/projects/${slug "${home}/${project}"}/memory";
  # updated by the privileged account with no switch
  xb = "/var/lib/xb/current";
}
