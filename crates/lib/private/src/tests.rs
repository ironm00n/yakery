use std::{
    fs::{self, File, Permissions},
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::{Duration, SystemTime},
};

use rustix::io::Errno;
use tempfile::TempDir;

use crate::{Dir, Mode, PARTIAL_EXPIRY, Replace, Secret, hex, random, secret::PASSWORD_CLASSES};

const NAMES: [&str; 3] = [".cache", "app", "files"];

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

fn error(made: anyhow::Result<Dir>) -> String {
    format!("{:#}", made.err().unwrap())
}

#[test]
fn names_are_created_private_and_files_are_never_replaced_unless_asked() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &NAMES).unwrap();
    assert!(dir.existing("a.png").unwrap().is_none());
    assert!(dir.write("a.png", b"one", Mode::Private, Replace::No).unwrap());
    let path = home.path().join(".cache/app/files/a.png");
    for chain in [".cache", ".cache/app", ".cache/app/files"] {
        assert_eq!(mode(&home.path().join(chain)), 0o700, "{chain}");
    }
    assert_eq!((mode(&path), fs::read(&path).unwrap()), (0o600, b"one".to_vec()));
    let again = Dir::open(home.path(), &NAMES).unwrap();
    assert_eq!(again.existing("a.png").unwrap(), Some(path.clone()));
    assert!(!again.write("a.png", b"two", Mode::Private, Replace::No).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"one");
    assert!(again.write("a.png", b"two", Mode::Readable, Replace::Yes).unwrap());
    assert_eq!((mode(&path), fs::read(&path).unwrap()), (0o644, b"two".to_vec()));
    symlink(&path, path.with_file_name("link")).unwrap();
    assert!(again.existing("link").is_err() && again.read("link").is_err());
    assert_eq!(entries(&home.path().join(".cache/app/files")), ["a.png", "link"]);
}

#[test]
fn a_symlinked_or_shared_name_on_the_way_is_refused() {
    let home = TempDir::new().unwrap();
    let elsewhere = home.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("app")).unwrap();
    symlink(&elsewhere, home.path().join(".cache")).unwrap();
    let linked = error(Dir::open(home.path(), &NAMES));
    assert!(linked.ends_with(".cache: not a directory, or a symlink"), "{linked}");
    assert!(!elsewhere.join("app/files").exists());
    fs::remove_file(home.path().join(".cache")).unwrap();
    let ours = home.path().join(".cache/app");
    fs::create_dir_all(&ours).unwrap();
    fs::set_permissions(&ours, Permissions::from_mode(0o770)).unwrap();
    let shared = error(Dir::open(home.path(), &NAMES));
    assert!(shared.ends_with("app: others may write to it (mode 770)"), "{shared}");
    assert!(!ours.join("files").exists());
    fs::set_permissions(&ours, Permissions::from_mode(0o750)).unwrap();
    assert!(Dir::open(home.path(), &NAMES).is_ok(), "only writing is refused to others");
}

#[test]
fn moving_a_name_mid_run_does_not_redirect_writes_or_reads() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &NAMES).unwrap();
    dir.save("state.json", &[1, 2]).unwrap();
    fs::rename(home.path().join(".cache"), home.path().join("moved")).unwrap();
    fs::create_dir_all(home.path().join("decoy/app/files")).unwrap();
    fs::write(home.path().join("decoy/app/files/state.json"), "[3]").unwrap();
    symlink(home.path().join("decoy"), home.path().join(".cache")).unwrap();
    dir.write("a.png", b"one", Mode::Private, Replace::No).unwrap();
    assert!(home.path().join("moved/app/files/a.png").exists());
    assert!(!home.path().join("decoy/app/files/a.png").exists());
    assert_eq!(dir.load::<Vec<u8>>("state.json").unwrap(), [1, 2]);
    assert!(dir.load::<Vec<u8>>("absent.json").unwrap().is_empty());
}

#[test]
fn a_killed_write_leaves_a_partial_file_that_only_a_later_sweep_drops() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &NAMES).unwrap();
    let at = home.path().join(".cache/app/files");
    let partial = at.join(".partial-killed");
    fs::write(&partial, b"on").unwrap();
    dir.write("a.png", b"one", Mode::Private, Replace::No).unwrap();
    dir.drop_stale_partials().unwrap();
    assert_eq!(entries(&at), [".partial-killed", "a.png"]);
    let old = SystemTime::now() - PARTIAL_EXPIRY - Duration::from_secs(60);
    File::options().write(true).open(&partial).unwrap().set_modified(old).unwrap();
    dir.drop_stale_partials().unwrap();
    assert_eq!(entries(&at), ["a.png"]);
}

#[test]
fn secrets_never_print_and_parse_only_as_hex_of_their_length() {
    assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(..)");
    assert_eq!(Secret::new("a"), Secret::new("a"));
    assert_ne!(Secret::new("a"), Secret::new("ab"));
    let token = random::token(32).unwrap();
    assert_eq!(Secret::hex(&token.expose().to_uppercase(), 32).unwrap(), token);
    assert!(token.expose().bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_ne!(token, random::token(32).unwrap());
    for refused in ["abc", &"g".repeat(64), &"a".repeat(62)] {
        let e = format!("{:#}", Secret::hex(refused, 32).unwrap_err());
        assert!(!e.contains(refused), "{e}");
    }
    assert_eq!(
        (hex::encode(&[0, 171]), hex::decode("00Ab").unwrap()),
        ("00ab".into(), vec![0, 171])
    );
}

#[test]
fn passwords_hold_every_class() {
    let all = PASSWORD_CLASSES.concat();
    for _ in 0..200 {
        let password = Secret::password(40).unwrap();
        let bytes = password.expose().as_bytes();
        assert_eq!(bytes.len(), 40);
        assert!(PASSWORD_CLASSES.iter().all(|class| bytes.iter().any(|b| class.contains(b))));
        assert!(bytes.iter().all(|b| all.contains(b)));
    }
    assert_ne!(Secret::password(40).unwrap(), Secret::password(40).unwrap());
}

#[test]
fn draws_stay_in_range() {
    assert!((0..1000).all(|_| random::below(3).unwrap() < 3));
    assert!(random::below(0).is_err());
    let word = random::chars(b"ab", 64).unwrap();
    assert!(word.len() == 64 && word.contains('a') && word.contains('b') && !word.contains('c'));
}

#[test]
fn where_rename_takes_no_flags_a_link_keeps_replace_no() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &NAMES).unwrap();
    let at = home.path().join(".cache/app/files");
    let nfs = |_: &str, _: &str| Err(Errno::INVAL);
    assert!(dir.write_with("a", b"one", Mode::Readable, Replace::No, nfs).unwrap());
    assert!(!dir.write_with("a", b"two", Mode::Private, Replace::No, nfs).unwrap());
    assert_eq!((fs::read(at.join("a")).unwrap(), mode(&at.join("a"))), (b"one".to_vec(), 0o644));
    let refused = |_: &str, _: &str| Err(Errno::ACCESS);
    assert!(dir.write_with("b", b"one", Mode::Private, Replace::No, refused).is_err());
    assert_eq!(entries(&at), ["a"], "no partial file stays behind");
}

#[test]
fn a_dir_made_here_is_new_and_private_and_appends_and_opens_stay_inside_it() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &[]).unwrap();
    let run = dir.create_dir("run-1").unwrap().unwrap();
    assert_eq!(mode(&home.path().join("run-1")), 0o700);
    assert!(dir.create_dir("run-1").unwrap().is_none(), "a name is never reused");
    symlink(home.path(), home.path().join("elsewhere")).unwrap();
    fs::write(home.path().join("file"), "").unwrap();
    assert!(dir.create_dir("elsewhere").is_err() && dir.create_dir("file").is_err());
    run.append("log.jsonl", b"{}\n").unwrap();
    run.append("log.jsonl", b"[]\n").unwrap();
    let log = home.path().join("run-1/log.jsonl");
    assert_eq!((fs::read_to_string(&log).unwrap(), mode(&log)), ("{}\n[]\n".into(), 0o600));
    let mut text = String::new();
    run.open_file("log.jsonl").unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "{}\n[]\n");
    assert!(run.open_file("absent").is_err() && !home.path().join("run-1/absent").exists());
    symlink(&log, home.path().join("run-1/link")).unwrap();
    assert!(run.open_file("link").is_err() && run.append("link", b"x").is_err());
    assert_eq!(fs::read_to_string(&log).unwrap(), "{}\n[]\n");
}

#[test]
fn a_subdir_is_opened_with_opens_checks_and_never_made() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &[]).unwrap();
    assert!(dir.subdir("run").unwrap().is_none() && !home.path().join("run").exists());
    fs::create_dir(home.path().join("run")).unwrap();
    fs::set_permissions(home.path().join("run"), Permissions::from_mode(0o700)).unwrap();
    dir.subdir("run").unwrap().unwrap().write("a", b"one", Mode::Private, Replace::No).unwrap();
    assert_eq!(fs::read(home.path().join("run/a")).unwrap(), b"one");
    fs::set_permissions(home.path().join("run"), Permissions::from_mode(0o770)).unwrap();
    let shared = format!("{:#}", dir.subdir("run").err().unwrap());
    assert!(shared.ends_with("run: others may write to it (mode 770)"), "{shared}");
    symlink(home.path().join("run"), home.path().join("link")).unwrap();
    let linked = format!("{:#}", dir.subdir("link").err().unwrap());
    assert!(linked.ends_with("link: not a directory, or a symlink"), "{linked}");
}

#[test]
fn partial_names_are_dirs_own_and_the_sweep_takes_only_old_partial_files() {
    let home = TempDir::new().unwrap();
    let dir = Dir::open(home.path(), &[]).unwrap();
    assert!(dir.write(".partial-x", b"", Mode::Private, Replace::No).is_err());
    assert!(dir.append(".partial-x", b"").is_err() && dir.file(".partial-x").is_err());
    assert!(dir.create_dir(".partial-x").is_err() && entries(home.path()).is_empty());
    let kept = home.path().join(".partial-dir");
    fs::create_dir(&kept).unwrap();
    let old = SystemTime::now() - PARTIAL_EXPIRY - Duration::from_secs(60);
    File::open(&kept).unwrap().set_modified(old).unwrap();
    dir.drop_stale_partials().unwrap();
    assert!(kept.exists(), "a directory is never swept");
}
