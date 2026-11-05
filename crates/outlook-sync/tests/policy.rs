use super::*;

#[test]
fn each_route_is_one_path_under_the_public_prefix() {
    assert_eq!(POLICY.url(Route::Callback), "https://schedule.example.test/neu/callback");
    assert_eq!(
        POLICY.url(Route::Notices(Feed::Lifecycle)),
        "https://schedule.example.test/neu/lifecycle"
    );
    assert_eq!(POLICY.prefix(), "/neu");
    assert_eq!(POLICY.host(), "schedule.example.test");
    assert_eq!(Route::of("/notify"), Some(Route::Notices(Feed::Changes)));
    assert_eq!(Route::of("/callback/"), None);
}
