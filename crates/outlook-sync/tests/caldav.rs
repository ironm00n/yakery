use super::*;

fn entry(file: &str, etag: Option<&str>, gone: bool) -> Entry {
    Entry { file: file.into(), etag: etag.map(str::to_string), gone }
}

#[test]
fn a_sync_collection_reads_as_files_etags_removals_and_the_next_token() {
    // as Stalwart 0.16.27 answered a sync-collection on 11-04 (push-3.log), plus a removal and a d: prefix
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/cal/owen%40example.test/neu/</D:href></D:response><D:response><D:href>/dav/cal/owen%40example.test/neu/ab.ics</D:href><D:propstat><D:prop><D:getetag>&quot;1&quot;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><d:response><d:href>/x/cd.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response><D:sync-token>https://x/sync/42</D:sync-token></D:multistatus>"#;
    let (entries, token) = responses(xml);
    assert_eq!(entries, [entry("ab.ics", Some("\"1\""), false), entry("cd.ics", None, true)]);
    assert_eq!(token.as_deref(), Some("https://x/sync/42"));
}
