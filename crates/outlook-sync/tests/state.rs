use std::pin::pin;
use std::task::Poll;

use jiff::ToSpan;
use serde_json::{Value, json};
use tokio::task::LocalSet;

use super::*;

fn login(dir: &Dir) -> Value {
    dir.load(LOGIN).unwrap()
}

#[test]
fn a_waiting_sign_in_is_taken_by_one_return_and_left_waiting_by_a_stale_one() {
    let temp = tempfile::tempdir().unwrap();
    let dir = Dir::open(temp.path(), &[]).unwrap();
    let now: Timestamp = "2026-11-06T12:00:00Z".parse().unwrap();
    let taken = |query: &str, at: Timestamp| {
        let pkce = take(&dir, &Returned::read(query), at).unwrap();
        pkce.map(|pkce| pkce.verifier().expose().to_owned())
    };
    assert_eq!(taken("code=c&state=s1", now), Err(Untaken::Nothing));
    let waiting =
        Login::Pending { state: Secret::new("s1"), verifier: Secret::new("v"), until: now };
    dir.save(LOGIN, &Some(waiting)).unwrap();
    assert_eq!(taken("code=c&state=s2", now), Err(Untaken::Stale));
    assert_eq!(taken("code=c&state=s1", now + 1.second()), Err(Untaken::Stale));
    assert_eq!(taken("code=c&state=s1", now), Ok("v".into()));
    assert_eq!(taken("code=c&state=s1", now), Err(Untaken::Taken));
}

#[test]
fn a_sign_in_serves_only_a_policy_whose_scope_it_asked_for() {
    let temp = tempfile::tempdir().unwrap();
    let dir = Dir::open(temp.path(), &[]).unwrap();
    let read = "openid offline_access https://graph.microsoft.com/Calendars.Read";
    let write = "openid offline_access https://graph.microsoft.com/Calendars.ReadWrite";
    let saved = |scope: &str| -> Tokens {
        let signed_in = Some(SignedIn { refresh: Secret::new("r"), scope: scope.into() });
        dir.save(TOKENS, &Tokens { signed_in }).unwrap();
        dir.load(TOKENS).unwrap()
    };
    let scope = |tokens: Tokens, needs| tokens.signed_in_for(needs).map(|s| s.scope);
    assert_eq!(scope(saved(write), write), Ok(write.into()));
    let short = format!("the sign-in asked for `{read}`, not `{write}`; run `outlook-sync login`");
    assert_eq!(scope(saved(read), write), Err(short));
    dir.save(TOKENS, &serde_json::json!({ "refresh": "r" })).unwrap();
    let unsigned = Err("not signed in; run `outlook-sync login`".into());
    assert_eq!(scope(dir.load(TOKENS).unwrap(), read), unsigned);
}

#[tokio::test]
async fn a_redeem_ends_the_sign_in_whether_or_not_its_connection_stays() {
    let temp = tempfile::tempdir().unwrap();
    let dir = Rc::new(Dir::open(temp.path(), &[]).unwrap());
    let (local, redeeming) = (LocalSet::new(), Some(Login::Redeeming { state: Secret::new("s") }));
    dir.save(LOGIN, &redeeming).unwrap();
    let refused = finish(dir.clone(), Secret::new("s"), async { Err("refused".into()) });
    assert_eq!(local.run_until(refused).await, Err("refused".into()));
    assert_eq!(login(&dir), json!({ "Done": { "Err": "refused" } }));
    dir.save(LOGIN, &redeeming).unwrap();
    let (answer, answered) = tokio::sync::oneshot::channel();
    local
        .run_until(async {
            let answer = async { answered.await.unwrap() };
            let mut dropped = pin!(finish(dir.clone(), Secret::new("s"), answer));
            std::future::poll_fn(|cx| {
                assert!(dropped.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        })
        .await;
    assert_eq!(login(&dir), json!({ "Redeeming": { "state": "s" } }));
    assert!(answer.send(Ok(())).is_ok(), "the redeem was dropped with its connection");
    local.await;
    assert_eq!(login(&dir), json!({ "Done": { "Ok": null } }));
}

#[tokio::test]
async fn a_redeem_ending_after_a_newer_login_leaves_that_sign_in_as_it_is() {
    let temp = tempfile::tempdir().unwrap();
    let dir = Rc::new(Dir::open(temp.path(), &[]).unwrap());
    let until = "2026-11-06T12:00:00Z";
    let pending = |state: &str, verifier: &str| Login::Pending {
        state: Secret::new(state),
        verifier: Secret::new(verifier),
        until: until.parse().unwrap(),
    };
    let rows = [
        (
            pending("s2", "v2"),
            json!({ "Pending": { "state": "s2", "verifier": "v2", "until": until } }),
        ),
        (Login::Redeeming { state: Secret::new("s2") }, json!({ "Redeeming": { "state": "s2" } })),
        (Login::Done(Ok(())), json!({ "Done": { "Ok": null } })),
    ];
    let (mut got, mut want) = (vec![], vec![]);
    for (newer, left) in rows {
        dir.save(LOGIN, &Some(pending("s1", "v1"))).unwrap();
        let returned = Returned::read("code=c&state=s1");
        let pkce = take(&dir, &returned, until.parse().unwrap()).unwrap().unwrap();
        dir.save(LOGIN, &Some(newer)).unwrap();
        let late = finish(dir.clone(), pkce.state().clone(), async { Err("late".into()) });
        got.push((LocalSet::new().run_until(late).await, login(&dir)));
        want.push((Err("late".into()), left));
    }
    assert_eq!(got, want);
}

#[test]
fn a_sign_in_left_redeeming_is_failed_as_the_daemon_starts() {
    let temp = tempfile::tempdir().unwrap();
    let dir = Dir::open(temp.path(), &[]).unwrap();
    let abandoned = |left: Option<Login>| {
        dir.save(LOGIN, &left).unwrap();
        abandon(&dir).unwrap();
        login(&dir)
    };
    let redeeming = Login::Redeeming { state: Secret::new("s") };
    assert_eq!(abandoned(Some(redeeming)), json!({ "Done": { "Err": ABANDONED } }));
    assert_eq!(abandoned(Some(Login::Done(Ok(())))), json!({ "Done": { "Ok": null } }));
    assert_eq!(abandoned(None), Value::Null);
    let until = "2026-11-06T12:00:00Z";
    let waiting = Login::Pending {
        state: Secret::new("s"),
        verifier: Secret::new("v"),
        until: until.parse().unwrap(),
    };
    let pending = json!({ "Pending": { "state": "s", "verifier": "v", "until": until } });
    assert_eq!(abandoned(Some(waiting)), pending);
}
