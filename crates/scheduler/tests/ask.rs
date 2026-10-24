use super::*;

#[test]
fn the_largest_link_page_fits_in_an_answer() {
    let start: Timestamp = "2026-10-26T00:00:00Z".parse().unwrap();
    let quarters = (0..57 * 96).map(|i| start + SignedDuration::from_mins(15 * i)).collect();
    let view = LinkView {
        title: Some("t".repeat(100)),
        duration: SignedDuration::from_mins(15),
        state: Shown::Open(quarters),
    };
    let bytes = serde_json::to_vec(&FrontAnswer::Link(view)).unwrap().len() as u64;
    assert!(bytes < MOST_ANSWERED, "{bytes}");
}
