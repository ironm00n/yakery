use super::*;

#[test]
fn only_a_state_change_naming_the_owner_wakes_the_loop() {
    // the forms Stalwart 0.16.27 sends
    let owners = "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"h\":{\"CalendarEvent\":\"saq\"}}}\n\n";
    let others = "event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"g\":{\"CalendarEvent\":\"sam\"}}}\n\n";
    assert!(changed(owners, "h"));
    assert!(!changed(others, "h"));
    assert!(!changed("event: ping\ndata: {\"interval\": 60}\n\n", "h"));
}
