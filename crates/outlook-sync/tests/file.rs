use super::*;

#[test]
fn a_graph_id_names_its_file_by_its_sha256() {
    // sha256sum of the Graph id
    let stem = "1b58086e633eb81838b55b82576398909f15bedd2bb17209b4eed1d9313e85ff";
    let name = FileName::of("AAMkAD=/x+y");
    assert_eq!(name.to_string(), format!("{stem}.ics"));
    assert_eq!(name.uid("schedule.example.test"), format!("{stem}@schedule.example.test"));
}

#[test]
fn only_a_non_empty_ics_name_in_the_collection_is_a_file_name() {
    assert_eq!("ab.ics".parse::<FileName>().map(|n| n.stem().to_string()), Ok("ab".into()));
    for bad in ["ab", ".ics", "a/b.ics", "ab.ics.bak"] {
        assert!(bad.parse::<FileName>().is_err(), "{bad}");
    }
}

#[test]
fn a_record_keyed_by_file_name_keeps_its_json_form() {
    let map: std::collections::BTreeMap<FileName, u8> =
        serde_json::from_str(r#"{"ab.ics": 1}"#).unwrap();
    assert_eq!(serde_json::to_string(&map).unwrap(), r#"{"ab.ics":1}"#);
}
