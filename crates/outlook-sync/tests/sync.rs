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
