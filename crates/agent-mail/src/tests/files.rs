use std::{
    fs::{File, Permissions},
    os::unix::fs::{PermissionsExt, symlink},
    time::SystemTime,
};

use super::*;
use crate::{
    cmd::read::folder,
    private::{PARTIAL_EXPIRY, PASSWORD_CLASSES, Secret},
    store::{Access, PasswordFile, Store, Tokens},
};

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

#[test]
fn the_attachments_folder_is_created_private_and_files_are_never_replaced() {
    let home = Scratch::new("folder");
    let dir = folder(&home.0, "Mx1").unwrap();
    assert!(dir.existing("a.png").unwrap().is_none());
    assert!(dir.write("a.png", b"one", false).unwrap());
    let path = home.0.join(".cache/agent-mail/attachments/Mx1/a.png");
    for chain in [
        ".cache",
        ".cache/agent-mail",
        ".cache/agent-mail/attachments",
        ".cache/agent-mail/attachments/Mx1",
    ] {
        assert_eq!(mode(&home.0.join(chain)), 0o700, "{chain}");
    }
    assert_eq!((mode(&path), fs::read(&path).unwrap()), (0o600, b"one".to_vec()));
    let again = folder(&home.0, "Mx1").unwrap();
    assert_eq!(again.existing("a.png").unwrap(), Some(path.clone()));
    assert!(!again.write("a.png", b"two", false).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"one");
    symlink(&path, path.with_file_name("link")).unwrap();
    assert!(again.existing("link").is_err());
}

#[test]
fn a_symlinked_or_shared_folder_on_the_way_is_refused() {
    let home = Scratch::new("refused");
    let elsewhere = home.0.join("elsewhere");
    fs::create_dir_all(elsewhere.join("agent-mail")).unwrap();
    symlink(&elsewhere, home.0.join(".cache")).unwrap();
    let linked = format!("{:#}", folder(&home.0, "Mx1").err().unwrap());
    assert!(linked.ends_with(".cache: not a directory, or a symlink"), "{linked}");
    assert!(!elsewhere.join("agent-mail/attachments").exists());
    fs::remove_file(home.0.join(".cache")).unwrap();
    let ours = home.0.join(".cache/agent-mail");
    fs::create_dir_all(&ours).unwrap();
    fs::set_permissions(&ours, Permissions::from_mode(0o770)).unwrap();
    let shared = format!("{:#}", folder(&home.0, "Mx1").err().unwrap());
    assert!(shared.ends_with("agent-mail: others may write to it (mode 770)"), "{shared}");
    assert!(!ours.join("attachments").exists());
    fs::set_permissions(&ours, Permissions::from_mode(0o750)).unwrap();
    assert!(folder(&home.0, "Mx1").is_ok(), "only writing is refused to others");
}

#[test]
fn moving_a_folder_mid_run_does_not_redirect_writes() {
    let home = Scratch::new("moved");
    let dir = folder(&home.0, "Mx1").unwrap();
    fs::rename(home.0.join(".cache"), home.0.join("moved")).unwrap();
    fs::create_dir_all(home.0.join("decoy/agent-mail/attachments/Mx1")).unwrap();
    symlink(home.0.join("decoy"), home.0.join(".cache")).unwrap();
    dir.write("a.png", b"one", false).unwrap();
    assert!(home.0.join("moved/agent-mail/attachments/Mx1/a.png").exists());
    assert!(!home.0.join("decoy/agent-mail/attachments/Mx1/a.png").exists());
}

#[test]
fn a_killed_write_leaves_a_partial_file_that_only_a_later_run_drops() {
    let home = Scratch::new("killed");
    let at = home.0.join(".cache/agent-mail/attachments/Mx1");
    folder(&home.0, "Mx1").unwrap();
    let partial = at.join(".partial-killed");
    fs::write(&partial, b"on").unwrap();
    folder(&home.0, "Mx1").unwrap().write("a.png", b"one", false).unwrap();
    assert_eq!(entries(&at), [".partial-killed", "a.png"]);
    expire(&partial);
    folder(&home.0, "Mx1").unwrap();
    assert_eq!(entries(&at), ["a.png"]);
}

fn expire(partial: &Path) {
    let old = SystemTime::now() - PARTIAL_EXPIRY - Duration::from_secs(60);
    File::options().write(true).open(partial).unwrap().set_modified(old).unwrap();
}

fn store(dir: &Scratch, login: &str) -> Store {
    Store { state: dir.0.join("state"), locks: dir.0.join("locks"), login: super::login(login) }
}

fn tokens(access: &str) -> Tokens {
    let access = Access { token: Secret::new(access), expires_at: 5 };
    Tokens { access: Some(access), refresh: Some(Secret::new("r")) }
}

#[tokio::test]
async fn tokens_are_private_and_kept_with_their_login() {
    let dir = Scratch::new("tokens");
    let agents = store(&dir, "agent@example.org");
    let held = agents.held().await.unwrap();
    assert!(agents.tokens(&held).unwrap().access.is_none());
    agents.save(tokens("a"), &held).unwrap();
    let back = agents.tokens(&held).unwrap();
    assert_eq!(
        (back.access.unwrap().token, back.refresh),
        (Secret::new("a"), Some(Secret::new("r")))
    );
    for (path, want) in [
        ("state/agent-mail", 0o700),
        ("locks/agent-mail/locks", 0o700),
        ("state/agent-mail/tokens.json", 0o600),
    ] {
        assert_eq!(mode(&dir.0.join(path)), want, "{path}");
    }
    assert_eq!(entries(&dir.0.join("state/agent-mail")), ["tokens.json"]);
    drop(held);
    let ada = store(&dir, "ada@agent.example.org");
    let held = ada.held().await.unwrap();
    assert!(ada.tokens(&held).unwrap().refresh.is_none());
    ada.save(tokens("adas"), &held).unwrap();
    drop(held);
    let again = store(&dir, "Agent@Example.org");
    assert!(again.tokens(&again.held().await.unwrap()).unwrap().access.is_none());
}

#[tokio::test]
async fn a_password_is_staged_beside_its_file_and_promoted_once() {
    let dir = Scratch::new("staged");
    fs::write(dir.0.join("pw"), "old").unwrap();
    let (file, auth) = (PasswordFile(dir.0.join("pw")), store(&dir, "ada@agent.example.org"));
    let held = auth.held().await.unwrap();
    let (a, b) = (Secret::new("new-a"), Secret::new("new-b"));
    let killed = dir.0.join(".partial-killed");
    fs::write(&killed, b"new-0").unwrap();
    expire(&killed);
    file.stage(&a, &held).unwrap();
    assert!(!killed.exists(), "a killed rotation's partial goes once it expires");
    assert_eq!((mode(&dir.0.join("pw.new")), dir.file("pw.new")), (0o600, Some("new-a".into())));
    file.stage(&b, &held).unwrap();
    assert!(file.promote(&a, &held).is_err());
    assert_eq!((dir.file("pw"), dir.file("pw.new")), (Some("old".into()), Some("new-b".into())));
    file.promote(&b, &held).unwrap();
    file.promote(&b, &held).unwrap();
    assert!(file.promote(&Secret::new("other"), &held).is_err());
    assert_eq!((dir.file("pw"), dir.file("pw.new")), (Some("new-b".into()), None));
}

#[test]
fn passwords_hold_every_class_and_secrets_never_print() {
    for _ in 0..200 {
        let password = Secret::password(40).unwrap();
        let bytes = password.expose().as_bytes();
        assert_eq!(bytes.len(), 40);
        assert!(PASSWORD_CLASSES.iter().all(|class| bytes.iter().any(|b| class.contains(b))));
        assert!(bytes.iter().all(|b| PASSWORD_CLASSES.concat().contains(b)));
    }
    assert_ne!(Secret::password(40).unwrap(), Secret::password(40).unwrap());
    assert_eq!(format!("{:?}", Secret::new("hunter2")), "Secret(..)");
}
