use super::*;

/// An event as iOS writes one: CRLF lines, a folded description, a TZID'd start.
fn ios(extra: &str) -> String {
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VTIMEZONE",
        "TZID:America/New_York",
        "END:VTIMEZONE",
        "BEGIN:VEVENT",
        "UID:ABC",
        "DTSTART;TZID=America/New_York:20261105T100000",
        "DTEND;TZID=America/New_York:20261105T113000",
        "SUMMARY:Coffee\\, then lab",
        "DESCRIPTION:line one\\nline",
        "  two",
        extra,
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .iter()
    .filter(|l| !l.is_empty())
    .map(|l| format!("{l}\r\n"))
    .collect()
}

#[test]
fn a_timed_event_keeps_its_zone_and_its_text_is_unescaped() {
    let event = graph(&ios("LOCATION:WVH 366")).unwrap();
    let ny = |t: &str| json!({"dateTime": t, "timeZone": "America/New_York"});
    assert_eq!(
        (event["start"].clone(), event["end"].clone()),
        (ny("2026-11-05T10:00:00"), ny("2026-11-05T11:30:00"))
    );
    assert_eq!(event["subject"], "Coffee, then lab");
    assert_eq!(event["body"]["content"], "line one\nline two");
    assert_eq!(event["location"]["displayName"], "WVH 366");
    assert_eq!((event["isAllDay"].clone(), event["showAs"].clone()), (json!(false), json!("busy")));
    assert!(event.get("recurrence").is_none());
}

#[test]
fn a_utc_time_goes_as_utc() {
    let utc = ios("")
        .replace("DTSTART;TZID=America/New_York:20261105T100000", "DTSTART:20261105T150000Z");
    assert_eq!(
        graph(&utc).unwrap()["start"],
        json!({"dateTime": "2026-11-05T15:00:00", "timeZone": "UTC"})
    );
}

#[test]
fn the_mirrors_own_files_read_back_as_they_were_written() {
    let mirrored = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261105\r\nDTEND;VALUE=DATE:20261106\r\nSUMMARY:Holiday\r\nTRANSP:TRANSPARENT\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let event = graph(mirrored).unwrap();
    assert_eq!(event["isAllDay"], json!(true));
    assert_eq!(event["start"]["dateTime"], "2026-11-05T00:00:00");
    assert_eq!(event["showAs"], "free");
}

#[test]
fn recurrences_with_a_graph_pattern_map_and_others_are_refused() {
    let rule = |r: &str| graph(&ios(&format!("RRULE:{r}"))).map(|e| e["recurrence"].clone());
    assert_eq!(
        rule("FREQ=WEEKLY;BYDAY=TU,TH;COUNT=10").unwrap(),
        json!({"pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["tuesday", "thursday"], "firstDayOfWeek": "monday"},
               "range": {"type": "numbered", "startDate": "2026-11-05", "numberOfOccurrences": 10}})
    );
    assert_eq!(rule("FREQ=WEEKLY").unwrap()["pattern"]["daysOfWeek"], json!(["thursday"]));
    assert_eq!(
        rule("FREQ=MONTHLY;BYDAY=-1FR;UNTIL=20270601T000000Z").unwrap(),
        json!({"pattern": {"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["friday"], "index": "last"},
               "range": {"type": "endDate", "startDate": "2026-11-05", "endDate": "2027-05-31"}})
    );
    assert_eq!(
        rule("FREQ=YEARLY;INTERVAL=2").unwrap()["pattern"],
        json!({"type": "absoluteYearly", "interval": 2, "month": 11, "dayOfMonth": 5})
    );
    assert_eq!(
        rule("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR").unwrap()["pattern"]["daysOfWeek"],
        json!(["monday", "tuesday", "wednesday", "thursday", "friday"])
    );
    assert!(rule("FREQ=MONTHLY;BYSETPOS=2;BYDAY=MO,TU").is_err());
    assert!(rule("FREQ=HOURLY").is_err());
    assert!(rule("FREQ=DAILY;INTERVAL=2;BYDAY=MO").is_err());
    assert!(rule("FREQ=MONTHLY;BYDAY=2FR;BYMONTHDAY=13").is_err());
    assert!(rule("FREQ=YEARLY;BYMONTHDAY=15").is_err());
    assert!(rule("FREQ=WEEKLY;BYMONTHDAY=15").is_err());
}

#[test]
fn an_evening_series_keeps_its_local_day() {
    let evening = |r: &str| {
        let ics = ios(&format!("RRULE:{r}"))
            .replace("20261105T100000", "20261105T210000")
            .replace("20261105T113000", "20261105T220000");
        graph(&ics).unwrap()["recurrence"].clone()
    };
    let weekly = evening("FREQ=WEEKLY;UNTIL=20261204T025959Z");
    assert_eq!(weekly["pattern"]["daysOfWeek"], json!(["thursday"]));
    assert_eq!(
        weekly["range"],
        json!({"type": "endDate", "startDate": "2026-11-05", "endDate": "2026-12-03"})
    );
}

#[test]
fn a_duration_stands_in_for_a_missing_end() {
    let timed =
        ios("").replace("DTEND;TZID=America/New_York:20261105T113000\r\n", "DURATION:PT45M\r\n");
    assert_eq!(graph(&timed).unwrap()["end"]["dateTime"], "2026-11-05T10:45:00");
    let all_day = "BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261105\r\nDURATION:P2D\r\nEND:VEVENT\r\n";
    assert_eq!(graph(all_day).unwrap()["end"]["dateTime"], "2026-11-07T00:00:00");
}

#[test]
fn a_floating_time_names_no_instant_and_is_refused() {
    let floating = ios("").replace("DTSTART;TZID=America/New_York:", "DTSTART:");
    assert!(graph(&floating).is_err());
}

#[test]
fn a_file_carrying_the_syncs_uid_for_its_name_never_goes_out_as_new() {
    let stem = "1b58086e633eb81838b55b82576398909f15bedd2bb17209b4eed1d9313e85ff";
    let file: FileName = format!("{stem}.ics").parse().unwrap();
    let ours = ios("").replace("UID:ABC", &format!("UID:{stem}@outlook-sync"));
    assert!(owned(&file, &ours).unwrap().is_none());
    let mine = owned(&file, &ios("")).unwrap().expect("the owner's");
    assert_eq!(mine.body()["transactionId"], stem);
}

#[test]
fn a_copy_of_the_syncs_file_under_another_name_is_the_owners() {
    let stem = "1b58086e633eb81838b55b82576398909f15bedd2bb17209b4eed1d9313e85ff";
    let ours = ios("").replace("UID:ABC", &format!("UID:{stem}@outlook-sync"));
    let copied = owned(&"copy.ics".parse().unwrap(), &ours).unwrap().expect("the owner's");
    assert_eq!(copied.body()["transactionId"], "copy");
}

#[test]
fn an_apps_uid_of_its_file_stem_at_its_own_domain_is_the_owners() {
    let file: FileName = "4A1B-77.ics".parse().unwrap();
    let app = ios("").replace("UID:ABC", "UID:4A1B-77@icloud.com");
    assert!(owned(&file, &app).unwrap().is_some());
}

#[test]
fn an_edit_sends_only_what_moved_from_the_version_the_sync_wrote() {
    let base = fields(&graph(&ios("")).unwrap());
    assert!(moved(&graph(&ios("")).unwrap(), &base).is_empty());
    let retitled = graph(&ios("").replace("SUMMARY:Coffee\\, then lab", "SUMMARY:Tea")).unwrap();
    assert_eq!(Value::Object(moved(&retitled, &base)), json!({"subject": "Tea"}));
}

#[test]
fn a_description_he_removes_goes_out_as_an_empty_body() {
    let base = fields(&graph(&ios("")).unwrap());
    let bare = graph(&ios("").replace("DESCRIPTION:line one\\nline\r\n  two\r\n", "")).unwrap();
    assert_eq!(
        Value::Object(moved(&bare, &base)),
        json!({"body": {"contentType": "text", "content": ""}})
    );
}
