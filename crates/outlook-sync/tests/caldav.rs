use super::*;

#[test]
fn hrefs_are_read_whatever_the_prefix_and_closing_tags_are_skipped() {
    // as Stalwart 0.16.27 answered a sync-collection on 11-04 (push-3.log), with a d: variant
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/cal/owen%40example.test/neu/</D:href></D:response><D:response><D:href>/dav/cal/owen%40example.test/neu/ab.ics</D:href><D:propstat><D:prop><D:getetag>&quot;1&quot;</D:getetag></D:prop></D:propstat></D:response><d:response><d:href>/x/cd.ics</d:href></d:response></D:multistatus>"#;
    assert_eq!(
        hrefs(xml).collect::<Vec<_>>(),
        [
            "/dav/cal/owen%40example.test/neu/",
            "/dav/cal/owen%40example.test/neu/ab.ics",
            "/x/cd.ics"
        ]
    );
}
