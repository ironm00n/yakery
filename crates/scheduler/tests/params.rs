use super::*;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).unwrap()
}

#[test]
fn a_query_reads_its_zone_week_and_slot() {
    let query = Query::parse(b"tz=America%2FChicago&week=2&slot=1761580800");
    assert_eq!(
        query,
        Query { tz: Some("America/Chicago".into()), week: 2, slot: Some(at(1761580800)) }
    );
}

#[test]
fn a_value_that_doesnt_parse_reads_as_absent_and_leaves_the_rest() {
    let query = Query::parse(b"tz=Europe%2FParis&week=next&slot=soon");
    assert_eq!(query, Query { tz: Some("Europe/Paris".into()), week: 0, slot: None });
}

#[test]
fn a_posted_form_reads_its_fields_slot_and_zone() {
    let posted =
        Posted::parse(b"slot=1761580800&tz=UTC&name=Ada+L&address=ada%40example.test&note=hi");
    let form = Form { name: "Ada L".into(), address: "ada@example.test".into(), note: "hi".into() };
    assert_eq!(posted, Posted { tz: Some("UTC".into()), slot: Some(at(1761580800)), form });
}

#[test]
fn a_slot_is_written_as_the_seconds_it_is_read_from() {
    assert_eq!(Slot(at(1761580800)).to_string(), "1761580800");
    assert_eq!("1761580800".parse(), Ok(Slot(at(1761580800))));
}
