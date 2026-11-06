use jiff::ToSpan;

use super::*;

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
