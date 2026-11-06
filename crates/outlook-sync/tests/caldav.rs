use super::*;

fn entry(file: &str, etag: Option<&str>, gone: bool) -> Entry {
    Entry { file: file.parse().unwrap(), etag: etag.map(str::to_string), gone }
}

#[test]
fn a_sync_collection_reads_as_files_etags_removals_and_the_next_token() {
    // Stalwart 0.16.27's answer to a sync-collection, plus a removal and a d: prefix
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/cal/owen%40example.test/neu/</D:href></D:response><D:response><D:href>/dav/cal/owen%40example.test/neu/ab.ics</D:href><D:propstat><D:prop><D:getetag>&quot;1&quot;</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response><d:response><d:href>/x/cd.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response><D:sync-token>https://x/sync/42</D:sync-token></D:multistatus>"#;
    let (entries, token) = responses(xml);
    assert_eq!(entries, [entry("ab.ics", Some("\"1\""), false), entry("cd.ics", None, true)]);
    assert_eq!(token.as_deref(), Some("https://x/sync/42"));
}

#[test]
fn a_property_a_file_lacks_is_not_its_removal() {
    // RFC 6578 §3.9's response, with calendar files' names
    let xml = r#"<?xml version="1.0" encoding="utf-8" ?>
   <D:multistatus xmlns:D="DAV:">
     <D:response>
       <D:href
   >http://webdav.example.com/home/cyrusdaboo/file.ics</D:href>
       <D:propstat>
         <D:prop>
           <D:getetag>"00004-abcd1"</D:getetag>
         </D:prop>
         <D:status>HTTP/1.1 200 OK</D:status>
       </D:propstat>
       <D:propstat>
         <D:prop>
           <R:bigbox xmlns:R="urn:ns.example.com:boxschema"/>
         </D:prop>
         <D:status>HTTP/1.1 404 Not Found</D:status>
       </D:propstat>
     </D:response>
     <D:response>
       <D:href
   >http://webdav.example.com/home/cyrusdaboo/test.ics</D:href>
       <D:status>HTTP/1.1 404 Not Found</D:status>
     </D:response>
     <D:sync-token>http://example.com/ns/sync/1238</D:sync-token>
   </D:multistatus>"#;
    let (entries, token) = responses(xml);
    assert_eq!(
        entries,
        [entry("file.ics", Some("\"00004-abcd1\""), false), entry("test.ics", None, true)]
    );
    assert_eq!(token.as_deref(), Some("http://example.com/ns/sync/1238"));
}

#[test]
fn a_namespace_declared_on_an_element_leaves_its_name() {
    let xml = r#"<multistatus xmlns="DAV:"><response><href>/x/ab.ics</href><propstat><prop><getetag xmlns="DAV:">"2"</getetag></prop></propstat></response></multistatus>"#;
    assert_eq!(responses(xml).0, [entry("ab.ics", Some("\"2\""), false)]);
}
