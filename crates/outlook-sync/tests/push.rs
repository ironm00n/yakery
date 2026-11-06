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

#[tokio::test(start_paused = true)]
async fn a_stream_silent_for_three_pings_is_dead() {
    let start = tokio::time::Instant::now();
    let silent = heard(std::future::pending::<reqwest::Result<()>>()).await.unwrap_err();
    assert_eq!(silent.to_string(), "no event or ping for 180 s");
    assert_eq!(start.elapsed(), Duration::from_secs(180));
    let late = heard(async {
        tokio::time::sleep(Duration::from_secs(179)).await;
        Ok("chunk")
    });
    assert_eq!(late.await.unwrap(), "chunk");
}
