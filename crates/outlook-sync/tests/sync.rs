use super::*;
use crate::policy::POLICY;

#[test]
fn a_listing_reaches_back_and_ahead_by_the_policy_days() {
    let now: Timestamp = "2026-11-04T12:00:00Z".parse().unwrap();
    let (from, to) = window(&POLICY, now);
    assert_eq!(from, "2026-10-28T12:00:00Z".parse().unwrap());
    assert_eq!(to, "2027-01-06T12:00:00Z".parse().unwrap());
    assert!(Graph::listing(from, to).contains("startDateTime=2026-10-28T12%3A00%3A00Z"));
}

#[test]
fn each_wake_asks_for_its_own_part_of_a_run() {
    let graph = Needs { graph: true, ..Needs::default() };
    let listing = Needs { listing: true, ..graph };
    assert_eq!(needs(&[Wake::Local]), Needs::default());
    assert_eq!(needs(&[Wake::Changed, Wake::Local]), graph);
    assert_eq!(needs(&[Wake::Reauthorize]), Needs { reauthorize: true, ..graph });
    assert_eq!(needs(&[Wake::Removed]), Needs { fresh_subscription: true, ..listing });
    for wake in [Wake::Daily, Wake::Missed, Wake::Refused, Wake::SignedIn] {
        assert_eq!(needs(&[Wake::Changed, wake]), listing, "{wake:?}");
    }
}
