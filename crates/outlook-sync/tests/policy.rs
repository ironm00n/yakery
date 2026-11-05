use super::*;

#[test]
fn each_route_is_one_path_under_the_public_prefix() {
    assert_eq!(POLICY.public.url(Route::Callback), "https://schedule.example.test/neu/callback");
    assert_eq!(
        POLICY.public.url(Route::Notices(Feed::Lifecycle)),
        "https://schedule.example.test/neu/lifecycle"
    );
    assert_eq!(POLICY.public.prefix(), "/neu");
    assert_eq!(Route::of("/notify"), Some(Route::Notices(Feed::Changes)));
    assert_eq!(Route::of("/callback/"), None);
}

#[test]
fn a_public_prefix_is_https_a_host_and_a_path_with_no_trailing_slash() {
    let public = |url: &str| Public::try_from(url.to_string());
    assert_eq!(
        public("https://h.example/a/b").unwrap().url(Route::Callback),
        "https://h.example/a/b/callback"
    );
    for bad in [
        "http://h.example/a",
        "https://h.example/a/",
        "https://h.example",
        "https:///a",
        "https://h.example/",
    ] {
        assert!(public(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_deployed_policy_parses() {
    if let Some(deployed) = option_env!("OUTLOOK_SYNC_POLICY") {
        serde_json::from_str::<Policy>(deployed).unwrap();
    }
}
