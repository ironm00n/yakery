use super::*;

fn entry(etag: Option<&str>, gone: bool) -> Entry {
    Entry { file: "f.ics".parse().unwrap(), etag: etag.map(Into::into), gone }
}

fn writing() -> Mirrored {
    Mirrored { id: "AAMk1".into(), etag: None, fields: Default::default(), content: None }
}

fn mirrored(etag: &str) -> Mirrored {
    Mirrored { etag: Some(etag.into()), content: Some("c1".into()), ..writing() }
}

#[test]
fn a_file_the_sync_wrote_and_no_one_touched_is_unchanged() {
    assert_eq!(plan(&entry(Some("\"7\""), false), Some(&mirrored("\"7\""))), Edit::Unchanged);
}

#[test]
fn a_newer_version_of_a_recorded_file_is_the_owners_edit() {
    assert_eq!(
        plan(&entry(Some("\"8\""), false), Some(&mirrored("\"7\""))),
        Edit::Edited(mirrored("\"7\""))
    );
}

#[test]
fn a_recorded_file_gone_is_the_owners_delete() {
    assert_eq!(
        plan(&entry(None, true), Some(&mirrored("\"7\""))),
        Edit::Deleted(mirrored("\"7\""))
    );
}

#[test]
fn a_file_with_no_record_is_new_and_one_gone_with_none_is_nothing() {
    assert_eq!(plan(&entry(Some("\"1\""), false), None), Edit::New);
    assert_eq!(plan(&entry(None, true), None), Edit::Unchanged);
}

#[test]
fn a_file_whose_write_was_cut_off_is_unsettled_whatever_its_etag() {
    let writing = writing();
    assert_eq!(plan(&entry(Some("\"8\""), false), Some(&writing)), Edit::Unsettled);
    assert_eq!(plan(&entry(None, true), Some(&writing)), Edit::Unsettled);
}

#[test]
fn only_the_syncs_settled_write_of_the_same_content_is_left_unwritten() {
    let etag = "\"7\"".to_string();
    assert!(untouched(Some(&mirrored("\"7\"")), Some(&etag), "c1"));
    assert!(!untouched(Some(&mirrored("\"7\"")), Some(&etag), "c2"), "Graph changed it");
    assert!(
        !untouched(Some(&mirrored("\"7\"")), Some(&"\"8\"".to_string()), "c1"),
        "the owner's edit"
    );
    assert!(!untouched(Some(&writing()), None, "c1"), "in flight");
    assert!(!untouched(None, Some(&etag), "c1"), "unrecorded");
    let owners = Mirrored { content: None, ..mirrored("\"7\"") };
    assert!(
        !untouched(Some(&owners), Some(&etag), "c1"),
        "the owner's version, recorded by edits()"
    );
}
