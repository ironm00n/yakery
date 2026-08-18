//! The §5 execution stack against a fake bootstrap store: `__ns __fetch` and
//! `__ns __build` for real (namespaces, overlay, pivot_root, nested pid ns,
//! the push phase's fd tricks), with `git` and `nix` replaced by busybox
//! scripts that record what they saw.
//!
//! Needs a static `slurm-ci` (the re-exec after `pivot_root` has no loader),
//! unprivileged user namespaces, and `SLURM_CI_TEST_BUSYBOX` pointing at a
//! static busybox. Skips itself otherwise.

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_slurm-ci");
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn env_dir() -> String {
    format!("/nix/store/{HASH}-ci-env")
}

fn has_interp(bin: &str) -> bool {
    let b = fs::read(bin).unwrap();
    let phoff = u64::from_le_bytes(b[0x20..0x28].try_into().unwrap()) as usize;
    let phentsize = u16::from_le_bytes(b[0x36..0x38].try_into().unwrap()) as usize;
    let phnum = u16::from_le_bytes(b[0x38..0x3a].try_into().unwrap()) as usize;
    (0..phnum).any(|i| u32::from_le_bytes(b[phoff + i * phentsize..][..4].try_into().unwrap()) == 3)
}

/// `None` with a reason when the environment cannot run the stack.
fn preflight() -> Result<PathBuf, String> {
    let busybox =
        std::env::var("SLURM_CI_TEST_BUSYBOX").map_err(|_| "SLURM_CI_TEST_BUSYBOX unset")?;
    if has_interp(BIN) {
        return Err("slurm-ci is dynamically linked".into());
    }
    let probe = Command::new(BIN)
        .args(["__ns", "/proc/self/exe", "--version"])
        .output()
        .map_err(|e| e.to_string())?;
    if !probe.status.success() {
        return Err(format!(
            "user namespaces unavailable: {}",
            String::from_utf8_lossy(&probe.stderr)
        ));
    }
    Ok(PathBuf::from(busybox))
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    bootstrap: PathBuf,
    ws: PathBuf,
    cache: PathBuf,
    key: PathBuf,
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Fixture {
    fn new(busybox: &Path) -> Self {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let root = dir.path().to_owned();
        let bootstrap = root.join("bootstrap-store.v1");
        let bin = bootstrap.join(format!("nix/store/{HASH}-ci-env/bin"));
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(bootstrap.join(format!("nix/store/{HASH}-ci-env/etc/ssl/certs")))
            .unwrap();
        fs::write(
            bootstrap.join(format!(
                "nix/store/{HASH}-ci-env/etc/ssl/certs/ca-bundle.crt"
            )),
            "",
        )
        .unwrap();
        fs::create_dir_all(bootstrap.join("nix/var/nix/db")).unwrap();
        fs::write(bootstrap.join("nix/var/nix/db/db.sqlite"), "").unwrap();
        symlink(env_dir(), bootstrap.join("env")).unwrap();
        fs::copy(busybox, bin.join("busybox")).unwrap();
        for tool in [
            "sh", "cat", "ls", "id", "mkdir", "echo", "touch", "chmod", "rm", "cp", "test", "wc",
        ] {
            symlink("busybox", bin.join(tool)).unwrap();
        }
        let sh = format!("#!{}/bin/sh\n", env_dir());
        write_exec(
            &bin.join("git"),
            &format!(
                r#"{sh}
[ "$1" = -C ] && shift 2
echo "git $*" >&2
case "$1" in
  init) mkdir -p "$3/.git" ;;
  fetch) [ -e /src/../FAIL_FETCH ] && exit 128; echo 'fake flake' > /src/flake.nix; [ -e /src/WANT_SUBMODULES ] && touch /src/.gitmodules ;;
  checkout) ;;
  rev-parse) if [ -e /src/WRONG_REV ]; then echo ffffffffffffffffffffffffffffffffffffffff; else echo {COMMIT}; fi ;;
esac
exit 0
"#
            ),
        );
        write_exec(
            &bin.join("nix"),
            &format!(
                r#"{sh}
echo "nix $*" >&2
case "$1" in
  build)
    echo "uid=$(id -u) TMPDIR=$TMPDIR HOME=$HOME"
    echo "nixconf=$(wc -l < /etc/nix/nix.conf)"
    echo "src=$(ls /src | tr '\n' ' ')"
    echo "pid=$$"
    mkdir -p /nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-built; echo out > /nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-built/f
    touch /tmp/scratch /xtmp/big
    [ -e /run/push/key ] && echo "KEY VISIBLE"
    echo probe > /cache/should-fail 2>/dev/null && echo "CACHE WRITABLE"
    [ -e /src/TAMPER ] && chmod u+w {env}/bin && echo pwned > {env}/bin/pwned
    [ -e /src/FAIL_BUILD ] && exit 1
    ;;
  path-info) echo "$3" ;;
  store) case "$2" in verify) [ -e /src/FAIL_VERIFY ] && exit 1 ;; sign) cat "$4" > /cache/signed-with ;; esac ;;
  copy) [ -e /src/FAIL_COPY ] && exit 1; echo copied > /cache/marker ;;
esac
exit 0
"#,
                env = env_dir()
            ),
        );
        let ws = root.join("ws");
        for sub in ["root", "upper", "work", "src", "xtmp"] {
            fs::create_dir_all(ws.join(sub)).unwrap();
        }
        let cache = root.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let key = root.join("key");
        fs::write(&key, "fake-signing-key").unwrap();
        Fixture {
            _dir: dir,
            root,
            bootstrap,
            ws,
            cache,
            key,
        }
    }

    fn fetch(&self) -> Output {
        Command::new(BIN)
            .args(["__ns", "__fetch", "--root"])
            .arg(self.ws.join("root"))
            .arg("--bootstrap")
            .arg(&self.bootstrap)
            .arg("--src")
            .arg(self.ws.join("src"))
            .args([
                "--url",
                "https://git.example/o/r.git",
                "--git-ref",
                "refs/heads/master",
                "--commit",
                COMMIT,
            ])
            .output()
            .unwrap()
    }

    fn build(&self, tmpdir: &str) -> Output {
        Command::new(BIN)
            .args(["__ns", "__build", "--root"])
            .arg(self.ws.join("root"))
            .arg("--bootstrap")
            .arg(&self.bootstrap)
            .arg("--upper")
            .arg(self.ws.join("upper"))
            .arg("--work")
            .arg(self.ws.join("work"))
            .arg("--src")
            .arg(self.ws.join("src"))
            .arg("--xtmp")
            .arg(self.ws.join("xtmp"))
            .arg("--cache")
            .arg(&self.cache)
            .arg("--key")
            .arg(&self.key)
            .args([
                "--pubkey",
                "dbp-ci-1:abc",
                "--cores",
                "2",
                "--jobs",
                "1",
                "--tmpdir",
                tmpdir,
                "--stats-file",
            ])
            .arg(self.ws.join("stats.json"))
            .args(["--", "packages.x86_64-linux.default"])
            .output()
            .unwrap()
    }

    fn reset_store(&self) {
        slurm_ci::cluster::ns::force_remove(&self.ws.join("upper")).unwrap();
        slurm_ci::cluster::ns::force_remove(&self.ws.join("work")).unwrap();
        fs::create_dir_all(self.ws.join("upper")).unwrap();
        fs::create_dir_all(self.ws.join("work")).unwrap();
        let _ = fs::remove_file(self.cache.join("marker"));
        let _ = fs::remove_file(self.cache.join("signed-with"));
    }

    fn marker(&self, name: &str) {
        fs::write(self.ws.join("src").join(name), "").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = slurm_ci::cluster::ns::force_remove(&self.root.join("ws"));
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn fetch_then_build_then_push() {
    let busybox = match preflight() {
        Ok(b) => b,
        Err(why) => {
            eprintln!("skipping: {why}");
            return;
        }
    };
    let f = Fixture::new(&busybox);

    let out = f.fetch();
    assert!(out.status.success(), "{}", text(&out));
    assert!(f.ws.join("src/flake.nix").exists());
    assert!(
        !f.ws.join("src/.git").exists(),
        ".git must be removed after the rev check"
    );

    let out = f.build("disk");
    let log = text(&out);
    assert_eq!(out.status.code(), Some(0), "{log}");
    assert!(
        log.contains("uid=1000 TMPDIR=/xtmp HOME=/tmp/home"),
        "{log}"
    );
    assert!(log.contains("nixconf=12"), "{log}");
    assert!(log.contains("src=flake.nix"), "{log}");
    assert!(
        log.contains("pid=2") || log.contains("pid=3"),
        "nix should run inside the nested pid namespace: {log}"
    );
    assert!(!log.contains("KEY VISIBLE"), "{log}");
    assert!(!log.contains("CACHE WRITABLE"), "{log}");
    assert!(log.contains("push: 1 paths pushed"), "{log}");
    assert!(f
        .ws
        .join("upper/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-built/f")
        .exists());
    assert!(f.ws.join("xtmp/big").exists());
    assert!(
        !f.ws.join("upper/tmp").exists(),
        "job tmpfs must not land in the store upper"
    );
    assert_eq!(
        fs::read_to_string(f.cache.join("marker")).unwrap().trim(),
        "copied"
    );
    assert_eq!(
        fs::read_to_string(f.cache.join("signed-with")).unwrap(),
        "fake-signing-key"
    );
    let stats: serde_json::Value =
        serde_json::from_slice(&fs::read(f.ws.join("stats.json")).unwrap()).unwrap();
    assert_eq!(stats["pushed_paths"], 1);

    f.reset_store();
    f.marker("TAMPER");
    let out = f.build("ram");
    let log = text(&out);
    assert_eq!(
        out.status.code(),
        Some(slurm_ci::verdict::exit::TAMPER),
        "{log}"
    );
    assert!(log.contains("TAMPER: lower store paths copied up"), "{log}");
    assert!(!f.cache.join("marker").exists(), "tamper must not push");
    fs::remove_file(f.ws.join("src/TAMPER")).unwrap();

    f.reset_store();
    f.marker("FAIL_VERIFY");
    let out = f.build("ram");
    assert_eq!(
        out.status.code(),
        Some(slurm_ci::verdict::exit::TAMPER),
        "{}",
        text(&out)
    );
    assert!(!f.cache.join("marker").exists());
    fs::remove_file(f.ws.join("src/FAIL_VERIFY")).unwrap();

    f.reset_store();
    f.marker("FAIL_COPY");
    let out = f.build("ram");
    assert_eq!(
        out.status.code(),
        Some(slurm_ci::verdict::exit::PUSH_FAILED),
        "{}",
        text(&out)
    );
    fs::remove_file(f.ws.join("src/FAIL_COPY")).unwrap();

    f.reset_store();
    f.marker("FAIL_BUILD");
    let out = f.build("ram");
    let log = text(&out);
    assert_eq!(out.status.code(), Some(1), "{log}");
    assert!(
        log.contains("push: 1 paths pushed"),
        "a red build still caches what it built: {log}"
    );
}

#[test]
fn fetch_refuses_wrong_rev_and_submodules() {
    let busybox = match preflight() {
        Ok(b) => b,
        Err(why) => {
            eprintln!("skipping: {why}");
            return;
        }
    };
    let f = Fixture::new(&busybox);
    f.marker("WRONG_REV");
    let out = f.fetch();
    assert!(!out.status.success());
    assert!(text(&out).contains("but the run pins"), "{}", text(&out));
    fs::remove_file(f.ws.join("src/WRONG_REV")).unwrap();

    f.marker("WANT_SUBMODULES");
    let out = f.fetch();
    assert!(!out.status.success());
    assert!(
        text(&out).contains("submodules are not supported"),
        "{}",
        text(&out)
    );
}
