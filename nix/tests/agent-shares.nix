{ pkgs, name }:
let
  shared = import ./agent-node.nix { inherit pkgs name; };
  checkout = "/home/ironmoon/workspace/redox";
  memory = "/home/ironmoon/.claude/projects/-home-ironmoon-workspace-redox/memory";
  lane = "${checkout}/claude/unboca";
  top = "/shr";
  share = "${top}/workspace/redox";
  shareMemory = "${top}/.claude/projects/-shr-workspace-redox/memory";
  # the memory dirs bound alone (shares.nix `memories`), his and as the accounts reach them
  memoryOnly = [
    {
      his = "/home/ironmoon/.claude/projects/-home-ironmoon-workspace-claude/memory";
      shared = "${top}/.claude/projects/-shr-workspace-claude/memory";
      checkout = "${top}/workspace/claude";
    }
    {
      his = "/home/ironmoon/.claude/projects/-home-ironmoon-workspace-autograder/memory";
      shared = "${top}/.claude/projects/-shr-workspace-autograder/memory";
      checkout = "${top}/workspace/autograder";
    }
  ];
  # his ACLs, as he sets them by hand on his tree: the lane's dir and the memory dir are the accounts' to write, with
  # what they make there his to write too, and the repo stores, recordings and audio are closed to them
  acls = pkgs.writeShellScript "acls" ''
    set -eu
    PATH=${pkgs.acl}/bin
    setfacl -R -P -m g:shr:rwX,d:g:shr:rwX,d:u:ironmoon:rwX ${lane} ${memory} ${
      pkgs.lib.concatMapStringsSep " " (m: m.his) memoryOnly
    }
    setfacl -m g:shr:--- ${checkout}/.jj ${checkout}/.git ${lane}/rec ${lane}/talk.m4a
  '';
in
pkgs.testers.runNixOSTest {
  name = "${name}-shares";

  nodes.desktop =
    { config, ... }:
    {
      imports = [
        shared.node
        ../../users/agent/shares.nix
      ];
      # the test VM's own mounts replace fileSystems
      virtualisation.fileSystems = config.agent.shares;
      users.users.eve.isNormalUser = true;
      environment.systemPackages = [ pkgs.acl ];
    };

  testScript =
    { nodes, ... }:
    shared.python
    + ''
      shares = ${builtins.toJSON (builtins.attrNames nodes.desktop.agent.shares)}
      memory_only = ${builtins.toJSON memoryOnly}
    ''
    + ''
      asEve = lambda cmd: "su eve -c " + shlex.quote(cmd)

      def done(cmd):
          code, out = desktop.execute(f"{cmd} 2>&1")
          assert code == 0, out

      booted(desktop)
      mounts = desktop.succeed("systemd-escape -p --suffix=mount " + shlex.join(shares)).split()
      assert len(mounts) == 2 + len(memory_only), mounts

      with subtest("a bind whose source is missing is skipped, and nothing is made in his home"):
          for unit in mounts:
              show = lambda prop: desktop.succeed(f"systemctl show -P {prop} {shlex.quote(unit)}").strip()
              assert show("ConditionTimestampMonotonic") != "0" and show("ConditionResult") == "no", unit
          desktop.fail("test -e /home/ironmoon/workspace || test -e /home/ironmoon/.claude")

      with subtest("he binds his checkout and memory dir under ${top}, which only root and the accounts traverse"):
          done(asHim(
              "mkdir -p ${lane}/rec ${checkout}/.jj ${checkout}/.git ${memory}"
              " && echo top > ${checkout}/README && echo lane > ${lane}/notes.md && echo voice > ${lane}/rec/a.txt"
              " && echo audio > ${lane}/talk.m4a && echo store > ${checkout}/.jj/x && echo store > ${checkout}/.git/x"
              " && echo mem > ${memory}/MEMORY.md"
              + "".join(f" && mkdir -p {m['his']} && echo mem > {m['his']}/MEMORY.md" for m in memory_only)
              + " && ${acls}"
          ))
          desktop.succeed("systemctl start " + shlex.join(mounts))
          assert desktop.succeed("stat -c '%U %G %a' ${top}").strip() == "root shr 710"
          for path in ["${share}", "${shareMemory}"] + [m["shared"] for m in memory_only]:
              assert desktop.succeed(f"stat -c %U {path}").strip() == "ironmoon", path

      with subtest("the account reads his checkout and writes in its lane and the memory dir"):
          assert desktop.succeed(asAgent("cat ${share}/README")).strip() == "top"
          assert desktop.succeed(asAgent("cat ${share}/claude/unboca/notes.md")).strip() == "lane"
          desktop.succeed(asAgent("echo a > ${share}/claude/unboca/new.md && echo a >> ${share}/claude/unboca/notes.md"))
          desktop.succeed(asAgent("mkdir ${share}/claude/unboca/sub && echo a > ${share}/claude/unboca/sub/f"))
          desktop.succeed(asAgent("echo a > ${shareMemory}/new.md && echo a >> ${shareMemory}/MEMORY.md"))

      with subtest("each memory dir bound alone is the account's to write, his tree beside it unbound"):
          for m in memory_only:
              desktop.succeed(asAgent(f"echo a >> {m['shared']}/MEMORY.md && echo a > {m['shared']}/new.md"))
              desktop.succeed(asHim(f"echo b >> {m['his']}/new.md"))
              assert desktop.succeed(asHim(f"cat {m['his']}/new.md")).strip() == "a\nb", m
              desktop.fail(f"test -e {m['checkout']}")

      with subtest("a mount made under ${top} does not reach his tree"):
          desktop.succeed("mount -t tmpfs none ${share}/claude/unboca/sub")
          desktop.fail("mountpoint -q ${lane}/sub")
          desktop.succeed("umount ${share}/claude/unboca/sub")

      with subtest("the account reads no recording, audio file or repo store, and writes nothing outside its lane"):
          for path in ["claude/unboca/rec/a.txt", "claude/unboca/talk.m4a", ".jj/x", ".git/x"]:
              refused(asAgent(f"cat ${share}/{path}"), "Permission denied")
          for path in ["claude/unboca/rec", ".jj", ".git"]:
              refused(asAgent(f"ls ${share}/{path}"), "Permission denied")
          refused(asAgent("echo a >> ${share}/README"), "Permission denied")
          refused(asAgent("echo a > ${share}/new"), "Permission denied")
          refused(asAgent("ls ${checkout}"), "Permission denied")

      with subtest("no third account traverses ${top}"):
          refused(asEve("ls ${top}"), "Permission denied")
          refused(asEve("cat ${share}/README"), "Permission denied")

      with subtest("his own access is unchanged, and what the account makes inherits his write"):
          desktop.succeed(asHim(
              "echo b >> ${checkout}/README && cat ${checkout}/.jj/x ${checkout}/.git/x ${lane}/rec/a.txt ${lane}/talk.m4a"
          ))
          for path in ["${lane}/new.md", "${lane}/sub/f", "${memory}/new.md"]:
              desktop.succeed(asHim(f"echo b >> {path}"))
          desktop.succeed(asHim("echo c > ${lane}/sub/g && rm ${lane}/new.md"))
          assert desktop.succeed("stat -c %U ${checkout} ${lane} ${memory}").split() == ["ironmoon"] * 3

      with subtest("a reboot keeps the binds and ${top}'s mode"):
          desktop.shutdown()
          booted(desktop)
          for unit in mounts:
              desktop.wait_for_unit(unit)
          assert desktop.succeed("stat -c '%U %G %a' ${top}").strip() == "root shr 710"
          assert desktop.succeed(asAgent("cat ${shareMemory}/new.md")).strip() == "a\nb"
          refused(asAgent("cat ${share}/.jj/x"), "Permission denied")
          refused(asEve("ls ${top}"), "Permission denied")
    '';
}
