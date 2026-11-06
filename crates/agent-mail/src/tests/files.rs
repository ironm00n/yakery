use std::{fs::File, os::unix::fs::PermissionsExt, time::SystemTime};

use private::{Mode, PARTIAL_EXPIRY, Replace, Secret, now};
use tokio::time::Instant;

use super::*;
use crate::{
    RECHECK,
    address::SessionId,
    cmd::{read::folder, wait},
    listeners::Heartbeat,
    show::utc,
    store::{PasswordFile, Tokens, Watched},
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

fn tokens(access: &str) -> Tokens {
    let access = oidc::Access { token: Secret::new(access), expires_at: Some(5) };
    Tokens { access: Some(access), refresh: Some(Secret::new("r")) }
}

#[tokio::test]
async fn tokens_saved_while_every_one_named_its_expiry_still_load() {
    let dir = Scratch::new("tokens-shape");
    let agents = dir.store("agent@example.org");
    let held = agents.held().await.unwrap();
    agents.save(Tokens::default(), &held).unwrap();
    let saved =
        r#"{"login":"agent@example.org","access":{"token":"a","expires_at":5},"refresh":"r"}"#;
    fs::write(dir.0.join("state/agent-mail/tokens.json"), saved).unwrap();
    let access = agents.tokens(&held).unwrap().access.unwrap();
    assert_eq!((access.token, access.expires_at), (Secret::new("a"), Some(5)));
}

#[tokio::test]
async fn tokens_are_private_and_kept_with_their_login() {
    let dir = Scratch::new("tokens");
    let agents = dir.store("agent@example.org");
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
    let ada = dir.store("ada@agent.example.org");
    let held = ada.held().await.unwrap();
    assert!(ada.tokens(&held).unwrap().refresh.is_none());
    ada.save(tokens("adas"), &held).unwrap();
    drop(held);
    let again = dir.store("Agent@Example.org");
    assert!(again.tokens(&again.held().await.unwrap()).unwrap().access.is_none());
}

/// What taking the auth lock in `dir` comes to, within an hour, and how many seconds that took.
async fn taking(dir: &Scratch) -> (u64, String) {
    let (agents, start) = (dir.store("agent@example.org"), Instant::now());
    let taken = tokio::time::timeout(Duration::from_secs(3600), agents.held()).await;
    let shown = taken.map_or("still waiting".into(), |t| shown(t.map(|_| "taken")));
    (
        start.elapsed().as_secs(),
        shown.replace(&dir.lock("auth.lock").display().to_string(), "auth.lock"),
    )
}

/// What taking the auth lock comes to while another process holds it under the lease `lease` writes.
async fn waited(lease: impl FnOnce(&Scratch)) -> (u64, String) {
    let dir = Scratch::new("lock");
    let _held = dir.store("agent@example.org").held().await.unwrap();
    lease(&dir);
    taking(&dir).await
}

const UNLEASED: &str =
    "error: auth.lock is held by a process that wrote no lease, so which one is not known";

#[tokio::test(start_paused = true)]
async fn a_lock_is_waited_for_while_its_lease_lasts_and_given_up_naming_its_holder_after() {
    let ended = now() - 1;
    let day = waited(|dir| dir.lease("auth.lock", 4242, now() + 86400)).await;
    assert_eq!(day, (3600, "still waiting".into()));
    assert_eq!(
        waited(|dir| dir.lease("auth.lock", 4242, ended)).await,
        (0, format!("error: auth.lock is held by pid 4242, past its lease until {}", utc(ended)))
    );
    let unsaid = waited(|dir| fs::write(dir.lock("auth.lock"), "").unwrap()).await;
    assert_eq!(unsaid, (0, UNLEASED.into()));
}

#[tokio::test(start_paused = true)]
async fn a_lock_let_go_leaves_no_lease_for_a_later_holder_that_writes_none() {
    let dir = Scratch::new("let-go");
    drop(dir.store("agent@example.org").held().await.unwrap());
    let unleased = File::open(dir.lock("auth.lock")).unwrap();
    unleased.lock().unwrap();
    assert_eq!(taking(&dir).await, (0, UNLEASED.into()));
}

#[tokio::test(start_paused = true)]
async fn a_waiter_takes_the_lock_once_its_holder_lets_go() {
    let dir = Scratch::new("released");
    let agents = dir.store("agent@example.org");
    let held = agents.held().await.unwrap();
    let start = Instant::now();
    let (taken, ()) = tokio::join!(agents.held(), async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        drop(held);
    });
    assert!(taken.is_ok());
    assert_eq!(start.elapsed().as_secs(), 60);
}

#[tokio::test(start_paused = true)]
async fn a_holder_leases_the_lock_for_each_step_and_gives_the_step_up_at_its_lease() {
    let dir = Scratch::new("lease");
    let lease = || {
        let said: Value =
            serde_json::from_str(&fs::read_to_string(dir.lock("auth.lock")).unwrap()).unwrap();
        (said["pid"].as_u64().unwrap(), said["until"].as_i64().unwrap())
    };
    let me = u64::from(std::process::id());
    let before = now();
    let held = dir.store("agent@example.org").held().await.unwrap();
    let taken = lease();
    let leased = held.within(Duration::from_secs(300), async { Ok(lease()) }).await.unwrap();
    let after = now();
    assert!(taken.0 == me && (before + 10..=after + 10).contains(&taken.1), "{taken:?}");
    assert!(leased.0 == me && (before + 300..=after + 300).contains(&leased.1), "{leased:?}");
    let start = Instant::now();
    let step = held.within(Duration::from_secs(300), std::future::pending::<Result<()>>());
    let given_up = tokio::time::timeout(Duration::from_secs(3600), step).await;
    let given_up = shown(given_up.expect("the step outlasted its lease").map(|()| ""));
    let lost =
        format!("error: gave up at the end of the lease on {}", dir.lock("auth.lock").display());
    assert_eq!((start.elapsed().as_secs(), given_up), (300, lost));
}

#[tokio::test]
async fn a_password_file_keeps_one_refusal_per_login_its_latest_state() {
    let dir = Scratch::new("refused");
    let file = PasswordFile(dir.0.join("pw"));
    let (ada, bob) = (dir.store("ada@agent.example.org"), dir.store("bob@agent.example.org"));
    let held = ada.held().await.unwrap();
    let states = ["one", "two!", "three"].map(|password| {
        fs::write(&file.0, password).unwrap();
        file.fingerprint(false).unwrap()
    });
    bob.refuse(&states[0], &held).unwrap();
    for state in &states {
        ada.refuse(state, &held).unwrap();
    }
    assert_eq!(states.each_ref().map(|s| ada.refused(s, &held).unwrap()), [false, false, true]);
    assert!(bob.refused(&states[0], &held).unwrap());
    let kept: Vec<Value> =
        serde_json::from_str(&dir.file("locks/agent-mail/locks/refused.json").unwrap()).unwrap();
    assert_eq!(kept.len(), 2);
}

#[tokio::test]
async fn a_password_is_staged_beside_its_file_and_promoted_once() {
    let dir = Scratch::new("staged");
    fs::write(dir.0.join("pw"), "old").unwrap();
    let (file, auth) = (PasswordFile(dir.0.join("pw")), dir.store("ada@agent.example.org"));
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
fn a_sessions_records_add_up_privately_and_stay_its_own() {
    let dir = Scratch::new("records");
    let records = dir.store("agent@example.org").records().unwrap();
    let (a, b): (SessionId, SessionId) = ("a-1".parse().unwrap(), "b-2".parse().unwrap());
    records.record(&a, vec!["pool".parse().unwrap()], vec![]).unwrap();
    records.record(&a, vec!["pool".parse().unwrap()], vec!["e1".into()]).unwrap();
    let Watched { topics, threads } = records.of(&a).unwrap();
    assert_eq!(
        (
            topics.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            threads.into_iter().collect::<Vec<_>>()
        ),
        (vec!["pool".into()], vec!["e1".into()])
    );
    assert!(records.of(&b).unwrap().topics.is_empty());
    let sessions = dir.0.join("locks/agent-mail/sessions");
    assert_eq!((mode(&sessions), mode(&sessions.join("a-1.json"))), (0o700, 0o600));
    rows!(parsed::<SessionId>; "../a", "", "a/b", &"a".repeat(65) => "refused"; "0f3c-9a" => "0f3c-9a");
}

#[test]
fn a_listener_counts_while_its_process_runs_and_it_scanned_within_three_rechecks() {
    let dir = Scratch::new("listeners");
    let listeners = dir.store("agent@example.org").listeners().unwrap();
    let ids: [SessionId; 3] = ["a-1", "b-2", "c-3"].map(|s| s.parse().unwrap());
    let topic = |t: &str| address(&format!("agent+{t}@example.org"));
    let (now, limit) = (1_800_000_000, 3 * RECHECK.as_secs() as i64);
    listeners.beat(&ids[0], &[topic("live")], &["e1".into()], now - limit).unwrap();
    listeners.beat(&ids[1], &[topic("stale")], &[], now - limit - 1).unwrap();
    let gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    drop(gone.wait_with_output().unwrap());
    let beat = Heartbeat {
        pid,
        topics: ["agent+gone@example.org".into()].into(),
        threads: [].into(),
        scanned: now,
    };
    listeners.put(&ids[2], beat).unwrap();
    assert_eq!(listeners.watched(now).unwrap(), [topic("live")]);
    let mine = &listeners.all().unwrap()["a-1"];
    assert_eq!(
        (mine.pid, mine.threads.iter().map(String::as_str).collect::<Vec<_>>()),
        (std::process::id(), vec!["e1"])
    );
    ids.iter().for_each(|id| listeners.leave(id).unwrap());
    assert_eq!(
        listeners.all().unwrap().keys().collect::<Vec<_>>(),
        ["c-3"],
        "another process's stays"
    );
}

#[test]
fn a_listener_leaving_keeps_the_heartbeat_a_newer_one_wrote_for_its_session() {
    let dir = Scratch::new("leave");
    let listeners = dir.store("agent@example.org").listeners().unwrap();
    let session: SessionId = "a-1".parse().unwrap();
    let newer = Heartbeat {
        pid: 1,
        topics: ["agent+pool@example.org".into()].into(),
        threads: [].into(),
        scanned: 0,
    };
    listeners.put(&session, newer.clone()).unwrap();
    listeners.leave(&session).unwrap();
    assert_eq!(listeners.all().unwrap().get("a-1"), Some(&newer));
}

#[test]
fn a_plain_wait_stands_down_only_for_its_own_live_listener_and_records_what_it_was_given() {
    let dir = Scratch::new("deferred");
    let store = dir.store("agent@example.org");
    let (listeners, records) = (store.listeners().unwrap(), store.records().unwrap());
    let (mine, other): (SessionId, SessionId) = ("a-1".parse().unwrap(), "b-2".parse().unwrap());
    let given = wait::Watching { topic: vec!["pool".parse().unwrap()], thread: vec!["e1".into()] };
    let (now, limit) = (1_800_000_000, 3 * RECHECK.as_secs() as i64);
    let deferred = || wait::deferred(&store, &mine, &given, now).unwrap();
    listeners.beat(&other, &[], &[], now).unwrap();
    assert_eq!(deferred(), None, "another session's listener");
    listeners.beat(&mine, &[], &[], now - limit - 1).unwrap();
    assert_eq!(deferred(), None, "a stale listener");
    let Watched { topics, threads } = records.of(&mine).unwrap();
    assert!(topics.is_empty() && threads.is_empty(), "a wait that runs records nothing");
    listeners.beat(&mine, &[], &[], now - limit).unwrap();
    assert_eq!(
        deferred().unwrap(),
        format!(
            "the mail-listener mod (pid {}) already waits for this session and hands its mail in as prompts; \
             session a-1 watches topics [pool] and the threads of [e1]",
            std::process::id()
        )
    );
    assert!(records.of(&other).unwrap().topics.is_empty());
}

#[test]
fn subscriptions_says_what_a_session_recorded_and_records_nothing() {
    let dir = Scratch::new("subscriptions");
    let records = dir.store("agent@example.org").records().unwrap();
    let (a, b): (SessionId, SessionId) = ("a-1".parse().unwrap(), "b-2".parse().unwrap());
    records
        .record(&a, vec!["pool".parse().unwrap(), "pyret".parse().unwrap()], vec!["e1".into()])
        .unwrap();
    rows!(|id: &SessionId| wait::watches(&records, id).unwrap();
        &a => "session a-1 watches topics [pool, pyret] and the threads of [e1]";
        &b => "session b-2 watches topics [] and the threads of []";
    );
    rows!(|id: &SessionId| wait::subscribed(&records, id, true).unwrap();
        &a => r#"{"topics":["pool","pyret"],"threads":["e1"]}"#;
        &b => r#"{"topics":[],"threads":[]}"#;
    );
    assert_eq!(
        wait::subscribed(&records, &a, false).unwrap(),
        "session a-1 watches topics [pool, pyret] and the threads of [e1]"
    );
    assert_eq!(entries(&dir.0.join("locks/agent-mail/sessions")), ["a-1.json", "lock"]);
}

#[test]
fn unsubscribe_takes_out_exactly_what_it_names_and_only_from_its_session() {
    let dir = Scratch::new("unsubscribe");
    let records = dir.store("agent@example.org").records().unwrap();
    let (a, b): (SessionId, SessionId) = ("a-1".parse().unwrap(), "b-2".parse().unwrap());
    let topics = || vec!["pool".parse().unwrap(), "pyret".parse().unwrap()];
    let threads = || vec!["e1".to_owned(), "e2".to_owned()];
    records.record(&a, topics(), threads()).unwrap();
    records.record(&b, topics(), threads()).unwrap();
    let named = wait::Watching {
        topic: vec!["pool".parse().unwrap(), "absent".parse().unwrap()],
        thread: vec!["e2".into(), "e9".into()],
    };
    assert_eq!(
        wait::forget(&records, &a, named).unwrap(),
        "session a-1 watches topics [pyret] and the threads of [e1]"
    );
    let absent = records
        .forget(&a, vec!["pyret".parse().unwrap(), "pool".parse().unwrap()], vec![])
        .unwrap();
    assert_eq!(
        wait::listed(&absent),
        "topics [pool] and the threads of []",
        "only what it held is gone"
    );
    assert_eq!(
        wait::watches(&records, &a).unwrap(),
        "session a-1 watches topics [] and the threads of [e1]"
    );
    assert_eq!(
        wait::watches(&records, &b).unwrap(),
        "session b-2 watches topics [pool, pyret] and the threads of [e1, e2]"
    );
}
