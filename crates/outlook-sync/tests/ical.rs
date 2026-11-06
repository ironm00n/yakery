use super::*;

const TEXTS: [&str; 5] =
    ["plain", "a, b; c \\ d", "line one\nline two\n", "Ünïcödé — 会議", "trailing backslash \\"];

#[test]
fn text_reads_back_as_it_was_written() {
    for text in TEXTS {
        assert_eq!(unescape(&escape(text)), text, "{text:?}");
    }
}

#[test]
fn a_folded_line_reads_back_whole_with_no_physical_line_over_75_octets() {
    let value = format!("{}{}", "x".repeat(70), "会議".repeat(40));
    let folded = fold(&format!("SUMMARY:{value}"));
    assert!(folded.split("\r\n").all(|l| l.len() <= 75), "{folded}");
    let read = lines(&folded);
    assert_eq!(
        (read.len(), read[0].name.as_str(), read[0].value.as_str()),
        (1, "SUMMARY", &*value)
    );
}

#[test]
fn transp_reads_back_as_free_or_busy() {
    for free_time in [true, false] {
        assert_eq!(free(Some(transp(free_time))), free_time);
    }
    assert!(!free(None));
}

#[test]
fn a_quoted_parameter_value_keeps_its_colon() {
    // RFC 5545 §3.2's example
    let read = lines(
        "DESCRIPTION;ALTREP=\"cid:part1.0001@example.org\":The Fall'98 Wild\r\n  Wizards Conference - - Las Vegas\\, NV\\, USA\r\n",
    );
    assert_eq!(read[0].params, [("ALTREP".to_string(), "cid:part1.0001@example.org".to_string())]);
    assert_eq!(
        unescape(&read[0].value),
        "The Fall'98 Wild Wizards Conference - - Las Vegas, NV, USA"
    );
}
