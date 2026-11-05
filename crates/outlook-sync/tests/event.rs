use serde_json::json;

use super::*;

fn now() -> Timestamp {
    "2026-11-04T12:00:00Z".parse().unwrap()
}

fn change(value: serde_json::Value) -> Change {
    serde_json::from_value(value).unwrap()
}

fn timed(extra: serde_json::Value) -> serde_json::Value {
    let mut event = json!({
        "id": "AAMkAD=/x+y",
        "subject": "Lab meeting",
        "start": {"dateTime": "2026-11-05T15:00:00.0000000", "timeZone": "UTC"},
        "end": {"dateTime": "2026-11-05T16:30:00.0000000", "timeZone": "UTC"},
        "isAllDay": false,
        "showAs": "busy",
    });
    event.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    event
}

/// The occurrence's file, unfolded: tests that read whole lines don't care where it folds.
fn rendered(value: serde_json::Value) -> String {
    let Change::Present(event) = change(value) else { panic!("not an occurrence") };
    event.ics(now()).unwrap().expect("not cancelled").replace("\r\n ", "")
}

#[test]
fn a_timed_occurrence_is_one_vevent_in_utc_named_by_its_graph_id() {
    let ics = rendered(timed(json!({})));
    let name = "1b58086e633eb81838b55b82576398909f15bedd2bb17209b4eed1d9313e85ff";
    for line in [
        format!("UID:{name}@outlook-sync"),
        "DTSTAMP:20261104T120000Z".into(),
        "DTSTART:20261105T150000Z".into(),
        "DTEND:20261105T163000Z".into(),
        "SUMMARY:Lab meeting".into(),
        "TRANSP:OPAQUE".into(),
    ] {
        assert!(ics.contains(&format!("{line}\r\n")), "{line} missing from\n{ics}");
    }
}

#[test]
fn the_file_the_sync_writes_reads_back_as_the_syncs_folded_as_written() {
    let Change::Present(event) = change(timed(json!({}))) else { panic!("not an occurrence") };
    let ics = event.ics(now()).unwrap().expect("not cancelled");
    assert!(ics.contains("\r\n "), "the UID line no longer folds:\n{ics}");
    let name = "1b58086e633eb81838b55b82576398909f15bedd2bb17209b4eed1d9313e85ff.ics";
    assert!(crate::local::stamped(&name.parse().unwrap(), &ics));
}

#[test]
fn all_day_free_tentative_and_private_carry_over() {
    let ics = rendered(timed(json!({
        "isAllDay": true,
        "start": {"dateTime": "2026-11-05T00:00:00.0000000", "timeZone": "UTC"},
        "end": {"dateTime": "2026-11-06T00:00:00.0000000", "timeZone": "UTC"},
        "showAs": "free",
        "sensitivity": "private",
    })));
    for line in [
        "DTSTART;VALUE=DATE:20261105",
        "DTEND;VALUE=DATE:20261106",
        "TRANSP:TRANSPARENT",
        "CLASS:PRIVATE",
    ] {
        assert!(ics.contains(&format!("{line}\r\n")), "{line} missing from\n{ics}");
    }
    assert!(rendered(timed(json!({"showAs": "tentative"}))).contains("STATUS:TENTATIVE\r\n"));
}

#[test]
fn cancelled_occurrences_and_removals_leave_the_mirror() {
    let Change::Present(cancelled) = change(timed(json!({"isCancelled": true}))) else { panic!() };
    assert!(cancelled.ics(now()).unwrap().is_none());
    let removed = change(json!({"id": "gone", "@removed": {"reason": "deleted"}}));
    assert!(matches!(removed, Change::Removed { id, .. } if id == "gone"));
}

#[test]
fn text_is_escaped_and_long_lines_fold_between_characters() {
    let subject = format!("a, b; c\\d\nnext {}", "é".repeat(60));
    let Change::Present(event) = change(timed(json!({"subject": subject}))) else { panic!() };
    let folded = event.ics(now()).unwrap().unwrap();
    for line in folded.split("\r\n") {
        assert!(line.len() <= 75, "{} octets: {line}", line.len());
    }
    let unfolded = folded.replace("\r\n ", "");
    assert!(
        unfolded.contains(&format!("SUMMARY:a\\, b\\; c\\\\d\\nnext {}", "é".repeat(60))),
        "{unfolded}"
    );
}

#[test]
fn a_time_outside_utc_is_refused_as_graph_sends_it() {
    let eastern =
        json!({"dateTime": "2026-11-05T15:00:00.0000000", "timeZone": "Eastern Standard Time"});
    assert!(serde_json::from_value::<Change>(timed(json!({"start": eastern}))).is_err());
}

#[test]
fn a_rewrite_of_an_unchanged_occurrence_differs_in_dtstamp_alone() {
    let Change::Present(event) = change(timed(json!({}))) else { panic!("not an occurrence") };
    let later: Timestamp = "2026-11-05T08:00:00Z".parse().unwrap();
    let (first, again) = (event.ics(now()).unwrap().unwrap(), event.ics(later).unwrap().unwrap());
    assert_ne!(first, again);
    assert_eq!(content(&first), content(&again));
    let Change::Present(moved) = change(timed(json!({"subject": "Lab meeting, moved"}))) else {
        panic!("not an occurrence")
    };
    assert_ne!(content(&moved.ics(now()).unwrap().unwrap()), content(&first));
}
