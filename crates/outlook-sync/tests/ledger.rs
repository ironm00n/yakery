use super::*;

fn entry(etag: Option<&str>, gone: bool) -> Entry {
    Entry { file: "f.ics".parse().unwrap(), etag: etag.map(Into::into), gone }
}

fn mirrored(etag: &str) -> Mirrored {
    Mirrored { id: "AAMk1".into(), etag: Some(etag.into()) }
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
    let writing = Mirrored { id: "AAMk1".into(), etag: None };
    assert_eq!(plan(&entry(Some("\"8\""), false), Some(&writing)), Edit::Unsettled);
    assert_eq!(plan(&entry(None, true), Some(&writing)), Edit::Unsettled);
}
