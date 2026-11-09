{
  pkgs,
  inputs,
  my-lib,
  name,
}:
let
  shared = import ./agent-node.nix { inherit pkgs name; };
  inherit (shared)
    accounts
    account
    uid
    connect
    listen
    hog
    ;
  keys = import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs;
  gpg = "${pkgs.gnupg}/bin/gpg";
  agent = "ironmoon's agent <agent@ironmoon.dev>";
  commit = pkgs.writeShellScript "commit" ''
    set -eu
    bin=/etc/profiles/per-user/${account.name}/bin
    mkdir ~/"$1"
    cd ~/"$1"
    $bin/git init -q git
    $bin/git -C git commit -q --allow-empty -m signed
    $bin/jj git init --colocate jj
    $bin/jj -R jj commit -m signed
    $bin/git -C git log -1 --format='%an <%ae>'
    $bin/jj -R jj log --no-graph -r @- -T 'author.name() ++ " <" ++ author.email() ++ ">\n"'
  '';
  common =
    { config, ... }:
    {
      imports = [
        shared.node
        inputs.home-manager.nixosModules.home-manager
        ../../users/agent/identity.nix
        ../../users/agent/forges.nix
        ../../users/agent/xb.nix
      ];
      home-manager = {
        useGlobalPkgs = true;
        useUserPackages = true;
        users = builtins.mapAttrs (_: _: { home.stateVersion = config.system.stateVersion; }) accounts;
      };
      programs.gnupg.agent.enable = true;
      environment.systemPackages = [
        pkgs.git
        pkgs.gnupg
      ];
    };
in
pkgs.testers.runNixOSTest {
  name = "${name}-account";
  node.specialArgs = { inherit my-lib; };

  nodes.desktop = common;
  nodes.laptop = {
    imports = [ common ];
    services.openssh.enable = true;
    users.users.${account.name}.openssh.authorizedKeys.keys = [ keys.snakeOilPublicKey ];
  };

  testScript = shared.python + ''
    # the fingerprints of a --with-colons listing's keys of one kind (sec, ssb or pub): each the first fpr after its key
    def fingerprints(listing, kind="sec"):
        found, wanted = [], False
        for fields in (line.split(":") for line in listing.splitlines()):
            if fields[0] in ("sec", "ssb", "pub", "sub"):
                wanted = fields[0] == kind
            elif fields[0] == "fpr" and wanted:
                found.append(fields[9])
                wanted = False
        return found

    def listing(homedir):
        return desktop.succeed(asHim(f"${gpg} --homedir {homedir} --batch --with-colons --list-secret-keys"))

    def gpgIn(homedir, args, passphrase=""):
        return desktop.succeed(asHim(f"install -d -m 700 {homedir} && ${gpg} --homedir {homedir} --batch --pinentry-mode loopback --passphrase {shlex.quote(passphrase)} {args}"))

    # a homedir's one key, all of it protected by the passphrase, if any
    def makeKey(homedir, uid, passphrase=""):
        gpgIn(homedir, f"--quick-gen-key {shlex.quote(uid)} ed25519 sign never", passphrase)
        [fpr] = fingerprints(listing(homedir))
        return fpr

    # a homedir's one key, unprotected, naming revoker as its designated revoker
    def makeRevocable(homedir, uid, revoker):
        name, email = uid[:-1].split(" <")
        params = f"%no-protection\nKey-Type: eddsa\nKey-Curve: ed25519\nKey-Usage: sign\nName-Real: {name}\nName-Email: {email}\nRevoker: 22:{revoker}\n%commit\n"
        desktop.succeed(f"printf %s {shlex.quote(params)} > /tmp/params")
        gpgIn(homedir, "--gen-key /tmp/params")
        [fpr] = fingerprints(listing(homedir))
        return fpr

    def asAccount(args):
        return desktop.succeed(asHim(f"sudo -n -u ${account.name} ${gpg} --batch {args}"))

    def agentKeys():
        files = desktop.succeed("ls /home/${account.name}/.gnupg/private-keys-v1.d")
        return files, fingerprints(asAccount("--with-colons --list-secret-keys"))

    agentPid = lambda: desktop.succeed(asAgent("systemctl --user show gpg-agent.service -p MainPID --value")).strip()

    def adopt(homedir, fpr):
        return asHim(f"${account.name}-adopt-gpg-key {homedir} {shlex.quote(fpr)}")

    def refusedAdopt(homedir, fpr, reason):
        before = agentKeys()
        refused(adopt(homedir, fpr), reason)
        assert agentKeys() == before, (fpr, before, agentKeys())

    def commits(dir):
        authors = desktop.succeed(asAgent(f"${commit} {dir}")).splitlines()
        assert authors == ["${agent}"] * 2, authors

    def ring(name, export):
        desktop.succeed(f"install -d -m 700 /tmp/ring-{name} && {export} | ${gpg} --homedir /tmp/ring-{name} --import")

    # whether HEAD of its git repo, then its jj repo, verifies against the ring's keys
    def verifies(dir, name):
        repos = [f"/home/${account.name}/{dir}/{repo}" for repo in ("git", "jj")]
        return [desktop.execute(f"GNUPGHOME=/tmp/ring-{name} git -c safe.directory='*' -C {repo} verify-commit HEAD")[0] == 0 for repo in repos]

    booted(desktop, laptop)

    with subtest("its own key, made at boot, is its own and private"):
        desktop.wait_for_unit("${account.name}-ssh-key.service")
        desktop.succeed("[ \"$(stat -c '%U %a' /home/${account.name}/.ssh/id_ed25519)\" = '${account.name} 600' ]")

    with subtest("its secrets are its own alone, and git signs in to each forge as its account there with the token it holds"):
        secrets = "/home/${account.name}/${account.secrets}"
        desktop.succeed(f"[ \"$(stat -c '%U %a' {secrets})\" = '${account.name} 700' ]")
        refused(asHim(f"ls {secrets}"), "Permission denied")
        for host, user, token in (("code.ironmoon.dev", "agent", "forgejo-agent-pat"), ("github.com", "ironmoon-agent", "gh-agent-pat")):
            desktop.succeed(asAgent(f"echo {token}-value > {secrets}/{token}"))
            filled = desktop.succeed(asAgent(f"printf 'protocol=https\\nhost={host}\\n\\n' | /etc/profiles/per-user/${account.name}/bin/git credential fill"))
            assert f"username={user}\npassword={token}-value\n" in filled, filled

    with subtest("its sessions run whatever xb's deploy last put in place, and only he places it"):
        desktop.succeed("[ \"$(stat -c '%U %a' /var/lib/xb)\" = 'ironmoon 755' ]")
        refused(asAgent("mkdir /var/lib/xb/v0"), "Permission denied")
        session = lambda cmd: desktop.succeed(asAgent(f"systemd-run --user --wait --pipe --quiet sh -c {shlex.quote(cmd)}")).strip()
        for version in ("v1", "v2"):
            desktop.succeed(asHim(f"install -D -m 755 /dev/stdin /var/lib/xb/{version}/bin/xb <<< 'echo {version}' && ln -sfn {version} /var/lib/xb/current"))
            assert (got := session("xb")) == version, got
        assert (got := session("~/.local/state/xb/current/bin/xb")) == "v2", got

    with subtest("its signing key, made at boot, is its own and private, and its unit prints the public half alone and leaves nothing running"):
        keyUnit = lambda prop: f"systemctl show ${account.name}-gpg-key -p {prop} --value"
        desktop.wait_until_succeeds(f"[ \"$({keyUnit('ExecMainExitTimestampMonotonic')})\" != 0 ] && ! {keyUnit('ActiveState')} | grep -q activating")
        assert (got := [desktop.succeed(keyUnit(prop)).strip() for prop in ("ActiveState", "Result")]) == ["inactive", "success"], got
        desktop.succeed("! grep -qs ${account.name}-gpg-key /proc/[0-9]*/cgroup")
        private = "/home/${account.name}/.gnupg/private-keys-v1.d"
        desktop.succeed(f"[ \"$(stat -c '%U %a' {private})\" = '${account.name} 700' ]")
        modes = desktop.succeed(f"stat -c '%U %a' {private}/*.key").splitlines()
        assert modes and set(modes) == {"${account.name} 600"}, modes
        printed = desktop.succeed("journalctl -b -u ${account.name}-gpg-key -o cat")
        assert "BEGIN PGP PUBLIC KEY BLOCK" in printed and "PRIVATE" not in printed, printed

    with subtest("nothing in his home reaches it"):
        desktop.succeed("install -d -m 700 -o ironmoon /home/ironmoon/.ssh")
        desktop.succeed("install -m 600 -o ironmoon /dev/null /home/ironmoon/.ssh/id_ed25519")
        desktop.succeed("install -m 644 -o ironmoon /dev/null /home/ironmoon/notes.md")
        refused(asAgent("cat /home/ironmoon/.ssh/id_ed25519"), "Permission denied")
        refused(asAgent("cat /home/ironmoon/notes.md"), "Permission denied")

    with subtest("he becomes the agent; the agent becomes no one"):
        out = desktop.succeed("su ironmoon -c 'sudo -n -u ${account.name} id -un'").strip()
        assert out == "${account.name}", out
        rules = desktop.execute("sudo -l -U ${account.name}")[1]
        assert "is not allowed to run sudo" in rules, rules

    with subtest("the account reaches neither his signing key nor his gpg-agent, even started by him"):
        desktop.wait_for_unit("user@1000.service")
        desktop.succeed("su ironmoon -c '${gpg} --batch --pinentry-mode loopback --passphrase \"\" --quick-gen-key \"IRONM00N <me@ironmoon.dev>\" ed25519 sign never'")
        sock = desktop.succeed("su ironmoon -c '${pkgs.gnupg}/bin/gpgconf --list-dirs agent-socket'").strip()
        assert sock == "/run/user/1000/gnupg/S.gpg-agent", sock
        refused(asAgent(f"python3 ${connect} {sock}"), "Permission denied")
        listed = desktop.succeed("su ironmoon -c 'sudo -n -u ${account.name} ${gpg} --batch --list-secret-keys --with-colons' 2>&1")
        uids = [line.split(":")[9] for line in listed.splitlines() if line.startswith("uid:")]
        assert uids == ["${agent}"], listed

    with subtest("its commits, by git and by jj, name it and verify against its key and not his"):
        desktop.wait_for_file("/home/${account.name}/.config/jj/config.toml")
        desktop.wait_for_file("/home/${account.name}/.config/git/config")
        commits("generated")
        ring("generated", asAgent("${gpg} --armor --export"))
        ring("his", asHim("${gpg} --armor --export"))
        assert (got := verifies("generated", "generated")) == [True, True], got
        assert (got := verifies("generated", "his")) == [False, False], got
        sessionAgent = agentPid()
        assert sessionAgent != "0", sessionAgent

    with subtest("adopt-gpg-key takes a primary key's whole fingerprint alone, and changes no keyring on any other"):
        [his] = fingerprints(listing("/home/ironmoon/.gnupg"))
        moved = "/home/ironmoon/.local/share/agent-gnupg"
        fpr = makeRevocable(moved, "${agent}", his)
        gpgIn(moved, f"--quick-add-key {fpr} cv25519 encr never")
        [sub] = fingerprints(listing(moved), "ssb")
        for wrong in (sub, "0x" + fpr, fpr[-16:], fpr.lower(), "agent@ironmoon.dev"):
            refusedAdopt(moved, wrong, "isn't the whole fingerprint of one primary key")
        refusedAdopt(moved, fpr[:20], "holds no secret key")

    with subtest("adopt-gpg-key refuses a key without a valid uid whose address is the agent's"):
        noUid = "has no valid uid with <agent@ironmoon.dev>"
        refusedAdopt("/home/ironmoon/.gnupg", his, noUid)
        revoked = "/home/ironmoon/.local/share/revoked-gnupg"
        revokedFpr = makeKey(revoked, "x <x@ironmoon.dev>")
        for op in ("add", "revoke"):
            gpgIn(revoked, f"--quick-{op}-uid {revokedFpr} " + shlex.quote("${agent}"))
        refusedAdopt(revoked, revokedFpr, noUid)
        mixed = "/home/ironmoon/.local/share/mixed-gnupg"
        refusedAdopt(mixed, makeKey(mixed, "${agent} <me@ironmoon.dev>"), noUid)

    with subtest("adopt-gpg-key refuses a key any part of which needs a passphrase, even one his pinentry gives"):
        locked = "/home/ironmoon/.local/share/locked-gnupg"
        lockedFpr = makeKey(locked, "${agent}", "pw")
        desktop.succeed(asHim(f"printf pw > {locked}/pw && printf 'pinentry-mode loopback\\npassphrase-file {locked}/pw\\n' > {locked}/gpg.conf"))
        refusedAdopt(locked, lockedFpr, "needs a passphrase")
        part = "/home/ironmoon/.local/share/part-gnupg"
        partFpr = makeKey(part, "${agent}")
        gpgIn(part, f"--quick-add-key {partFpr} ed25519 sign never", "pw")
        refusedAdopt(part, partFpr, "needs a passphrase")

    with subtest("adopt-gpg-key refuses a key whose every secret part the homedir doesn't hold"):
        full, stub = "/home/ironmoon/.local/share/full-gnupg", "/home/ironmoon/.local/share/stub-gnupg"
        stubFpr = makeKey(full, "${agent}")
        gpgIn(full, f"--quick-add-key {stubFpr} ed25519 sign never")
        desktop.succeed(asHim(f"install -d -m 700 {stub} && ${gpg} --homedir {full} --batch --export-secret-subkeys {stubFpr} | ${gpg} --homedir {stub} --batch --import"))
        refusedAdopt(stub, stubFpr, f"{stub} doesn't hold every secret part of {stubFpr}")

    with subtest("adopt-gpg-key copies the agent's key, subkey, revoker and all, in place of every key the account holds, and its commits verify"):
        other = "/home/ironmoon/.local/share/other-gnupg"
        makeRevocable(other, "someone <someone@ironmoon.dev>", his)
        desktop.succeed(asHim(f"${gpg} --homedir {other} --batch --export-secret-keys | sudo -n -u ${account.name} ${gpg} --batch --import"))
        desktop.succeed(adopt(moved, fpr))
        assert (got := agentKeys()[1]) == [fpr], got
        assert (got := fingerprints(asAccount("--with-colons --list-keys"), "pub")) == [fpr], got
        desktop.succeed("systemctl restart ${account.name}-gpg-key")
        desktop.succeed("journalctl --sync")
        printed = desktop.succeed("journalctl -u ${account.name}-gpg-key -o cat")
        end = "-----END PGP PUBLIC KEY BLOCK-----"
        block = printed[printed.rindex("-----BEGIN PGP PUBLIC KEY BLOCK-----"):printed.rindex(end) + len(end)]
        shown = desktop.succeed(f"install -d -m 700 /tmp/shown && echo {shlex.quote(block)} | ${gpg} --homedir /tmp/shown --batch --with-colons --import-options show-only --import")
        assert (got := fingerprints(shown, "pub")) == [fpr], shown
        commits("moved")
        ring("moved", asHim(f"${gpg} --homedir {moved} --armor --export"))
        assert (got := verifies("moved", "moved")) == [True, True], got
        assert (got := verifies("moved", "generated")) == [False, False], got

    with subtest("its key unit and adopt-gpg-key leave the account's running gpg-agent alone"):
        assert (got := agentPid()) == sessionAgent, (got, sessionAgent)

    with subtest("adopt-gpg-key says so when the account's gpg imports nothing"):
        plain = "/home/ironmoon/.local/share/plain-gnupg"
        plainFpr = makeKey(plain, "${agent}")
        desktop.succeed("echo 'import-options show-only' > /home/${account.name}/.gnupg/gpg.conf")
        refused(adopt(plain, plainFpr), f"the agent account doesn't hold {plainFpr} alone")
        assert (got := agentKeys()[1]) == [], got
        desktop.succeed("rm /home/${account.name}/.gnupg/gpg.conf && systemctl restart ${account.name}-gpg-key")
        assert len(got := agentKeys()[1]) == 1, got

    with subtest("its key unit refuses to run while the account holds a second key, the agent's or not, and runs again once it holds one"):
        desktop.succeed(asHim(f"${gpg} --batch --export-secret-keys {his} | sudo -n -u ${account.name} ${gpg} --batch --import"))
        desktop.fail("systemctl restart ${account.name}-gpg-key")
        desktop.succeed("journalctl -u ${account.name}-gpg-key -o cat | grep -q '^2 secret keys, not '")
        asAccount(f"--yes --delete-secret-and-public-keys {his}")
        desktop.succeed("systemctl restart ${account.name}-gpg-key")

    with subtest("the account together is killed at its cap, and below it nothing is"):
        desktop.succeed(asAgent("systemd-run --user --scope python3 ${hog} 50"))
        desktop.fail(asAgent("systemd-run --user --scope python3 ${hog} 400"))
        assert sliceOoms() > 0

    with subtest("his own loopback services answer him and refuse the account"):
        desktop.succeed("su ironmoon -c 'python3 ${listen} 8384 >/dev/null 2>&1 &'")
        desktop.wait_until_succeeds("su ironmoon -c 'python3 ${connect} 8384'")
        refused(asAgent("python3 ${connect} 8384"), "Connection refused")

    with subtest("its key logs in to itself on the laptop and never to him"):
        laptop.wait_for_unit("sshd.service")
        desktop.succeed("install -m 600 -o ${account.name} ${keys.snakeOilPrivateKey} /home/${account.name}/.ssh/test_key")
        ssh = "ssh -i /home/${account.name}/.ssh/test_key -o StrictHostKeyChecking=no -o BatchMode=yes"
        out = desktop.succeed(asAgent(f"{ssh} ${account.name}@laptop id -un")).strip()
        assert out == "${account.name}", out
        refused(asAgent(f"{ssh} ironmoon@laptop true"), "Permission denied (publickey")
  '';
}
