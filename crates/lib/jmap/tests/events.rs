use super::*;

#[test]
fn the_stream_gives_each_event_once_its_blank_line_arrives() {
    let pushed = |chunks: &[&[u8]]| {
        let mut events = Events::default();
        chunks.iter().map(|c| events.push(c)).collect::<Vec<_>>()
    };
    let none: Vec<String> = vec![];
    assert_eq!(
        pushed(&[b"event: state\r\ndata: {}\r\n", b"\r\n"]),
        [none.clone(), vec!["event: state\ndata: {}".into()]]
    );
    assert_eq!(
        pushed(&[b"event: ping\ndata: {}\n\nevent: state\nda", b"ta: {}\n\n: c\n\n"]),
        [
            vec!["event: ping\ndata: {}".to_owned()],
            vec!["event: state\ndata: {}".into(), ": c".into()]
        ]
    );
    assert_eq!(pushed(&[b"data: \xc3", b"\xa9\n\n"]), [none, vec!["data: é".into()]]);
}

#[test]
fn an_event_is_a_ping_a_state_change_or_other_data() {
    let change = |account: &str, state: &str| {
        let types = HashMap::from([("CalendarEvent".to_owned(), state.to_owned())]);
        Some(Event::Changed(StateChange { changed: HashMap::from([(account.to_owned(), types)]) }))
    };
    let rows = [
        (
            "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"h\":{\"CalendarEvent\":\"saq\"}}}",
            change("h", "saq"),
        ),
        (
            "data: {\"@type\":\"StateChange\",\ndata: \"changed\":{\"g\":{\"CalendarEvent\":\"s\"}}}",
            change("g", "s"),
        ),
        ("event: ping\ndata: {\"interval\": 30}\n\n", Some(Event::Ping)),
        ("event: ping\nevent: state\ndata: {}", Some(Event::Other)),
        ("event: state\ndata: {\"@type\":\"StateChange\"}", Some(Event::Other)),
        ("data: {\"@type\":\"StateChange\",\"changed\":{\"h\":{\"Email\":1}}}", Some(Event::Other)),
        ("data: {}", Some(Event::Other)),
        ("data: not json", Some(Event::Other)),
        ("data", Some(Event::Other)),
        (": data: {}", None),
        ("event: state", None),
        ("", None),
    ];
    for (text, event) in rows {
        assert_eq!(Event::read(text), event, "{text:?}");
    }
}
