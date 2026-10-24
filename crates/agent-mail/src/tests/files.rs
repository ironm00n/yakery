use std::{fs::File, os::unix::fs::PermissionsExt, time::SystemTime};

use private::{Mode, PARTIAL_EXPIRY, Replace, Secret};

use super::*;
use crate::{
    cmd::read::folder,
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
    assert!(dir.write("a.png", b"one", Mode::Private, Replace::No).unwrap());
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
    assert!(!again.write("a.png", b"two", Mode::Private, Replace::No).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"one");
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
