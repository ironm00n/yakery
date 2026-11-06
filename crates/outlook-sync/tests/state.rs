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
