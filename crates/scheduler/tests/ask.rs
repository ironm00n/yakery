use super::*;

#[test]
fn the_largest_link_page_fits_in_an_answer() {
    let start: Timestamp = "2026-10-26T00:00:00Z".parse().unwrap();
    let days = 7 * crate::link::MOST_WEEKS + 1;
    let per_day = 24 * 60 / crate::slots::GRID_MINUTES;
    let quarters = (0..days * per_day)
        .map(|i| start + SignedDuration::from_mins(crate::slots::GRID_MINUTES * i))
        .collect();
    let view = LinkView {
        title: Some("t".repeat(crate::link::MOST_TITLE)),
        duration: SignedDuration::from_mins(15),
        state: Shown::Open(quarters),
    };
    let bytes = serde_json::to_vec(&FrontAnswer::Link(view)).unwrap().len() as u64;
    assert!(bytes < MOST_ANSWERED, "{bytes}");
}
